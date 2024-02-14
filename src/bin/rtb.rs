use clap::Parser;
use diesel::connection::SimpleConnection;
use diesel::{Connection, RunQueryDsl, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use eyre::eyre;
use eyre::{ContextCompat, Report, Result, WrapErr};
use futures::stream::StreamExt;
use rtb::result_forest::ResultForest;
use rtb::schema;
use rtb::{roam, search};

use std::io::Write;
use std::path::PathBuf;

use tracing::{debug_span, info, info_span, instrument};

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
    Answer(Answer),
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
        .with_target(false)
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
            pragma temp_store = memory;
            pragma cache_size = -2000000; -- 2GB
            pragma mmap_size = 2000000;   -- 2GB
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
        Subcommand::Answer(answer) => exec_answer(&mut db_conn, &answer).await,
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

    // Delete any embeddings with no matching item.
    {
        let span = info_span!("Delete orphaned embeddings");
        let _guard = span.enter();
        let num_deleted = diesel::sql_query(
            "delete from item_embedding where not exists (select * from roam_item ri where ri.id = item_embedding.item_id);",
        )
        .execute(conn)
        .wrap_err("Failed to delete orphaned embeddings")?;
        info!(num_deleted, "Deleted orphaned embeddings");
    }

    Ok(())
}

#[derive(clap::Parser, Debug)]
struct UpdateEmbeddings {
    /// OpenAI API key.
    #[clap(long, env = "OPENAI_API_KEY")]
    openai_api_key: String,

    /// Delete all existing embeddings and re-generate.
    #[clap(long)]
    reset: bool,
}

#[instrument(skip_all)]
async fn exec_update_embeddings(
    conn: &mut SqliteConnection,
    args: &UpdateEmbeddings,
) -> Result<()> {
    // Create the OpenAI client.
    let openai_config =
        async_openai::config::OpenAIConfig::new().with_api_key(&args.openai_api_key);
    let openai_client = async_openai::Client::with_config(openai_config)
        .with_backoff(backoff::ExponentialBackoff::default());

    // Delete all existing embeddings if requested.
    if args.reset {
        let span = info_span!("Deleting existing embeddings");
        let _guard = span.enter();
        diesel::delete(schema::item_embedding::table)
            .execute(conn)
            .wrap_err("Failed to delete existing embeddings")?;
    }

    let mut embeddings_updated = 0;

    let batch_size = 512;
    let request_concurrency = 4;

    // Fetch item IDs which need to be embedded.
    #[derive(Clone, diesel::Queryable, diesel::QueryableByName)]
    struct ItemToEmbed {
        #[diesel(sql_type = diesel::sql_types::Text)]
        id: roam::BlockId,
    }
    let ids_to_embed = diesel::sql_query(
        "
        select id from roam_item 
        where 
            id not in (select item_id from item_embedding)
            and length(contents) > 0;
        ",
    )
    .load::<ItemToEmbed>(conn)
    .wrap_err("Failed to find Roam blocks that need embeddings")?;

    // Function to embed a batch of items.
    let process_batch = |batch: Vec<(roam::BlockId, String)>| {
        let openai_client = openai_client.clone();
        async move {
            // Request embeddings from OpenAI.
            let all_contents = batch
                .iter()
                .map(|(_, contents)| contents.as_str())
                .collect::<Vec<_>>();
            let all_ids = batch.iter().map(|(id, _)| id).collect::<Vec<_>>();
            let all_embeddings = rtb::embeddings::embed_text_batch(&openai_client, &all_contents)
                .await
                .wrap_err("Failed to request embeddings for batch")?;

            // Construct embedding records for each embedding in the batch.
            let item_embeddings = all_embeddings
                .into_iter()
                .enumerate()
                .map(move |(i, embedding)| rtb::db::ItemEmbedding {
                    item_id: *all_ids[i],
                    embedded_text: all_contents[i].to_string(),
                    embedding: embedding.clone(),
                })
                .collect::<Vec<_>>();

            Result::<_, Report>::Ok(item_embeddings)
        }
    };

    let items_to_embed = ids_to_embed
        .into_iter()
        .map(|item| -> Result<_> {
            let embed_contents = rtb::db::get_embeddable_text(conn, item.id)?;
            Ok((item.id, embed_contents))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut embedded_chunks = futures::stream::iter(items_to_embed.chunks(batch_size))
        .map(|batch| process_batch(batch.to_vec()))
        .buffer_unordered(request_concurrency);

    // loop over stream items
    while let Some(chunk) = embedded_chunks.next().await {
        // Insert the embeddings into the database.
        for item_embedding in chunk? {
            diesel::insert_into(schema::item_embedding::table)
                .values(&item_embedding)
                .on_conflict(schema::item_embedding::item_id)
                .do_update()
                .set(&item_embedding)
                .execute(conn)
                .wrap_err("Failed to insert item embedding")?;
            embeddings_updated += 1;
        }

        info!(
            embeddings_updated,
            total_to_embed = items_to_embed.len(),
            "Updated batch"
        );
    }

    Ok(())
}

#[derive(clap::Parser)]
struct Search {
    /// OpenAI API key.
    #[clap(long, env = "OPENAI_API_KEY")]
    openai_api_key: String,

    /// Return the top K results.
    #[clap(short, default_value("32"))]
    k: usize,

    /// The text to search for.
    query: String,

    /// Write output, formatted as a Roam bulleted list, to this file.
    #[clap(long, short('o'), default_value("/dev/stdout"))]
    output: PathBuf,
}

#[instrument(skip_all)]
async fn exec_search(conn: &mut SqliteConnection, args: &Search) -> Result<()> {
    // Embed the query.
    let openai_config =
        async_openai::config::OpenAIConfig::new().with_api_key(&args.openai_api_key);
    let openai_client = async_openai::Client::with_config(openai_config);
    let query_embedding = {
        let span = info_span!("Embed query");
        let _guard = span.enter();
        rtb::embeddings::embed_text(&openai_client, &args.query)
            .await
            .wrap_err("Failed to embed query")?
    };

    // Perform the similarity search.
    let k_most_similar: Vec<(search::Distance, roam::BlockId)> =
        search::SimilaritySearch::new(query_embedding)
            .with_top_k(args.k)
            .with_distance_metric(search::cosine_distance)
            .execute(conn)
            .await
            .wrap_err("Failed to execute similarity search")?;

    // Collect results into a result forest.
    let mut result_forest = ResultForest::new();
    for (distance, item_id) in &k_most_similar {
        result_forest
            .add_item(conn, *item_id, *distance)
            .wrap_err_with(|| format!("Failed to add item to result forest: {}", item_id))?;
    }

    // Open the output file and write the results, if set:
    let mut output_file = std::fs::File::create(&args.output)
        .wrap_err_with(|| format!("Failed to create output file {:?}", args.output))?;
    writeln!(output_file, "Query: `{}`", args.query)?;
    for subset_page in result_forest
        .get_subset_page_list(conn)
        .wrap_err("Failed to format result forest")?
    {
        writeln!(output_file, "{}", subset_page.to_roam_text(1))?;
    }

    Ok(())
}

#[derive(clap::Parser)]
struct Answer {
    /// OpenAI API key.
    #[clap(long, env = "OPENAI_API_KEY")]
    openai_api_key: String,

    /// Use the top N results to inform the answer.
    #[clap(short, default_value("512"))]
    n_results: usize,

    /// Write output, formatted as Roam markdown, to this file.
    #[clap(long, short('o'), default_value("/dev/stdout"))]
    output: PathBuf,

    /// The text to search for.
    query: String,
}

#[instrument(skip_all)]
async fn exec_answer(conn: &mut SqliteConnection, args: &Answer) -> Result<()> {
    // Embed the query.
    let openai_config =
        async_openai::config::OpenAIConfig::new().with_api_key(&args.openai_api_key);
    let openai_client = async_openai::Client::with_config(openai_config);
    let query_embedding = {
        let span = info_span!("Embed query");
        let _guard = span.enter();
        rtb::embeddings::embed_text(&openai_client, &args.query)
            .await
            .wrap_err("Failed to embed query")?
    };

    // Perform the similarity search.
    let k_most_similar: Vec<(search::Distance, roam::BlockId)> =
        search::SimilaritySearch::new(query_embedding)
            .with_top_k(args.n_results)
            .with_distance_metric(search::cosine_distance)
            .execute(conn)
            .await
            .wrap_err("Failed to execute similarity search")?;

    // Create a result forest from the search results.
    let mut result_forest = ResultForest::new();
    for (distance, item_id) in k_most_similar {
        result_forest
            .add_item(conn, item_id, distance)
            .wrap_err("Failed to add item to result forest")?;
    }

    // Generate the answer.
    let answer = {
        let span = info_span!("Generating response");
        let _guard = span.enter();
        rtb::prompting::generate_answer(conn, &openai_client, &result_forest, &args.query)
            .await
            .wrap_err("Failed to generate response.")?
    };

    // Write the answer to the output file.
    let mut output_file = std::fs::File::create(&args.output)
        .wrap_err_with(|| format!("Failed to create output file {:?}", args.output))?;
    writeln!(output_file, "Query: `{}` #GPT", args.query)?;
    writeln!(output_file, "{}", answer)?;

    Ok(())
}
