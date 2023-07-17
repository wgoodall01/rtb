use clap::Parser;
use diesel::connection::SimpleConnection;
use diesel::{Connection, RunQueryDsl, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use eyre::{ensure, eyre};
use eyre::{ContextCompat, Result, WrapErr};
use hora::core::ann_index::ANNIndex;
use rtb::schema;
use std::io::Write;
use std::path::PathBuf;

use tracing::{debug_span, info, info_span, instrument, trace};

/// Embed Diesel migrations into the binary.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

#[derive(clap::Parser)]
struct Args {
    /// Path to the database file.
    #[clap(long, default_value = "rtb.db")]
    db: PathBuf,

    /// Increase logging verbosity.
    #[clap(short, long)]
    verbose: bool,

    #[clap(subcommand)]
    cmd: Subcommand,
}

#[derive(clap::Parser)]
enum Subcommand {
    Import(Import),
    UpdateEmbeddings(UpdateEmbeddings),
    Search(Search),
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments.
    let args = Args::parse();

    // Configure tracing to show events, and to show info-level spans.
    let default_verbosity = if args.verbose {
        tracing_subscriber::filter::LevelFilter::DEBUG
    } else {
        tracing_subscriber::filter::LevelFilter::INFO
    };
    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(default_verbosity.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_span_events(
            tracing_subscriber::fmt::format::FmtSpan::CLOSE
                | tracing_subscriber::fmt::format::FmtSpan::NEW,
        )
        .init();

    // Connect to the database.
    let db_path_str = args
        .db
        .to_str()
        .wrap_err("Failed to convert database path to string")?;
    let mut db_conn = diesel::sqlite::SqliteConnection::establish(db_path_str)
        .wrap_err("Failed to connect to database.")?;

    // Set pragmas.
    {
        let span = debug_span!("Setting database pragmas");
        let _guard = span.enter();
        let query = "
            pragma foreign_keys = on;
            pragma journal_mode = wal;
            pragma auto_vacuum = incremental;
            pragma cache_size = -2000000 -- 2GB;
        ";
        db_conn
            .batch_execute(query)
            .wrap_err("Failed to set foreign keys pragma.")?;
    }

    // Run any pending Diesel migrations.
    {
        let span = debug_span!("Running pending database migrations");
        let _guard = span.enter();
        db_conn
            .run_pending_migrations(MIGRATIONS)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to run pending database migrations.")?;
    }

    // Execute the subcommand.
    let result = match args.cmd {
        Subcommand::Import(import) => exec_import(&mut db_conn, &import).await,
        Subcommand::UpdateEmbeddings(update_embeddings) => {
            exec_update_embeddings(&mut db_conn, &update_embeddings).await
        }
        Subcommand::Search(search) => exec_search(&mut db_conn, &search).await,
    };

    // Attempt to run 'pragma optimize'
    {
        let span = debug_span!("Running database optimization");
        let _guard = span.enter();
        let _ = db_conn.batch_execute("pragma optimize;");
    }

    result
}

#[derive(clap::Parser)]
struct Import {
    /// Path to the RoamResearch JSON export file to import.
    roam_json_export_file: PathBuf,
}

async fn exec_import(conn: &mut SqliteConnection, args: &Import) -> Result<()> {
    // Open the file.
    let file = std::fs::File::open(&args.roam_json_export_file)
        .wrap_err("Failed to open Roam export file")?;

    // Map it into memory.
    let mmap = unsafe {
        memmap::MmapOptions::new()
            .map(&file)
            .wrap_err("Failed to map Roam export file into memory")?
    };

    let export = {
        let _span = info_span!("Load RoamResearch export", file = ?args.roam_json_export_file);

        // Load the file into memory.
        let export: rtb::roam::Export =
            serde_json::from_slice(&mmap).wrap_err("Failed to parse Roam export file")?;

        export
    };

    // Count number of pages.
    let num_pages = export.pages.len();

    // Count number of children.
    fn count_children(child: &rtb::roam::Item) -> u64 {
        1 + child.children.iter().map(count_children).sum::<u64>()
    }
    let num_children = export
        .pages
        .iter()
        .flat_map(|p| p.children.iter())
        .map(count_children)
        .sum::<u64>();

    info!(
        num_pages = num_pages,
        num_children = num_children,
        "Loaded Roam export"
    );

    // Load the pages into the database.
    conn.transaction(|tx| -> Result<()> {
        let span = info_span!("Load export into database");
        let _guard = span.enter();

        let mut items_inserted = 0;
        for (i, page) in export.pages.iter().enumerate() {
            // Insert the page.
            items_inserted += rtb::db::insert_roam_page(tx, page)
                .wrap_err("Failed to insert page into database")?;

            if i % 256 == 0 {
                info!(
                    new_pages = i + 1,
                    new_items = items_inserted,
                    total_pages = export.pages.len(),
                );
            }
        }
        Ok(())
    })
    .wrap_err("Failed to load pages to database")?;

    Ok(())
}

#[derive(clap::Parser, Debug)]
struct UpdateEmbeddings {
    /// OpenAI API key.
    #[clap(long, env = "OPENAI_API_KEY")]
    openai_api_key: String,
}

#[instrument(skip_all)]
async fn exec_update_embeddings(
    conn: &mut SqliteConnection,
    args: &UpdateEmbeddings,
) -> Result<()> {
    // Create the OpenAI client.
    let openai_config =
        async_openai::config::OpenAIConfig::new().with_api_key(&args.openai_api_key);
    let openai_client = async_openai::Client::with_config(openai_config);

    let mut embeddings_computed = 0;
    let batch_size: i32 = 512;

    loop {
        // Fetch item IDs which need to be embedded.
        #[derive(diesel::Queryable, diesel::QueryableByName)]
        struct ItemToEmbed {
            #[diesel(sql_type = diesel::sql_types::Text)]
            id: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            contents: String,
        }
        let items_to_embed = diesel::sql_query(
            "
            select id, contents from roam_item 
            where 
                id not in (select item_id from item_embedding)
                and length(contents) > 0
            limit ?;
            ",
        )
        .bind::<diesel::sql_types::Integer, _>(batch_size)
        .load::<ItemToEmbed>(conn)
        .wrap_err("Failed to find Roam blocks that need embeddings")?;

        // If there are no more items to embed, we're done.
        if items_to_embed.is_empty() {
            break;
        }

        for item in &items_to_embed {
            trace!(id=%item.id, contents=%item.contents, "Embedding item");
        }

        // Embed the items.
        let all_item_ids: Vec<&str> = items_to_embed.iter().map(|i| i.id.as_str()).collect::<_>();
        let all_contents: Vec<&str> = items_to_embed.iter().map(|i| i.contents.as_str()).collect();
        let all_embeddings = rtb::embeddings::embed_text_batch(&openai_client, &all_contents)
            .await
            .wrap_err("Failed to embed batch")?;

        // Insert the embeddings into the database.
        for i in 0..all_item_ids.len() {
            let item_embedding = rtb::db::ItemEmbedding {
                item_id: all_item_ids[i].to_string(),
                embedded_text: all_contents[i].to_string(),
                embedding: all_embeddings[i].clone(),
            };

            diesel::insert_into(schema::item_embedding::table)
                .values(&item_embedding)
                .execute(conn)
                .wrap_err("Failed to insert item embedding")?;
        }

        // Update the count of embeddings computed.
        embeddings_computed += all_item_ids.len() as u64;

        info!(
            embeddings_computed,
            batch_size = all_item_ids.len(),
            "Embedded batch"
        )
    }

    Ok(())
}

#[derive(clap::Parser)]
struct Search {
    /// OpenAI API key.
    #[clap(long, env = "OPENAI_API_KEY")]
    openai_api_key: String,

    /// Return the top K results.
    #[clap(short, default_value("64"))]
    k: usize,

    /// The text to search for.
    query: String,

    /// Write output, formatted as a Roam bulleted list, to this file.
    #[clap(long, short('o'))]
    output: Option<PathBuf>,
}

#[instrument(skip_all)]
async fn exec_search(conn: &mut SqliteConnection, args: &Search) -> Result<()> {
    // Load all the item embeddings.
    let item_embeddings = {
        let span = info_span!("Load item embeddings");
        let _guard = span.enter();

        schema::item_embedding::table
            .load::<rtb::db::ItemEmbedding>(conn)
            .wrap_err("Failed to load all item embeddings")?
    };

    ensure!(
        !item_embeddings.is_empty(),
        "No item embeddings found in database"
    );

    // Create a vector similarity index.
    let mut index = {
        let span = info_span!("Add items to similarity index");
        let _guard = span.enter();
        let mut index = hora::index::hnsw_idx::HNSWIndex::<f32, String>::new(
            item_embeddings[0].embedding.dimensionality(),
            &hora::index::hnsw_params::HNSWParams::<f32>::default(),
        );
        for e in &item_embeddings {
            let vector: &[f32] = e.embedding.as_ref();
            index
                .add(vector, e.item_id.clone())
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to add vector to index")?
        }
        index
    };

    {
        let span = info_span!("Build similarity index");
        let _guard = span.enter();
        index
            .build(hora::core::metrics::Metric::Euclidean)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to build similarity index")?;
    }

    // Embed the query.
    let openai_config =
        async_openai::config::OpenAIConfig::new().with_api_key(&args.openai_api_key);
    let openai_client = async_openai::Client::with_config(openai_config);
    let query_embedding = rtb::embeddings::embed_text(&openai_client, &args.query)
        .await
        .wrap_err("Failed to embed query")?;

    // Search the index for similar items to the query.
    let search_results = index.search(query_embedding.as_ref(), args.k);

    // Print the results.
    info!(result_count = search_results.len());
    for result_id in &search_results {
        info!(result_id);
    }

    // Open the output file and write the results, if set:
    if let Some(output_file) = &args.output {
        let mut output_file = std::fs::File::create(output_file)
            .wrap_err_with(|| format!("Failed to create output file {:?}", args.output))?;

        writeln!(output_file, "Query: `{}`", args.query).wrap_err("Failed to write to output")?;
        for result_id in search_results {
            writeln!(output_file, "\t(({}))", result_id).wrap_err("Failed to write to output")?;
        }
    }

    Ok(())
}
