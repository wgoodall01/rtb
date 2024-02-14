use async_openai::types::Role;
use diesel::{QueryDsl, RunQueryDsl, SqliteConnection};
use eyre::{ContextCompat, Result, WrapErr};
use indoc::formatdoc;

use crate::{
    db,
    result_forest::{self, ResultForest},
    schema,
};

/// What model to use for question answering?
pub const CHAT_MODEL: &str = "gpt-4-1106-preview";

/// Generate an answer to a textual question.
pub async fn generate_answer(
    conn: &mut SqliteConnection,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    results: &ResultForest,
    question: &str,
) -> Result<String> {
    let mut prompt: Vec<(Role, String)> = vec![];

    prompt.push((
        Role::System,
        formatdoc! {"
            You are a helpful question-answering system named QAS. Your goal is to answer a factual question based on the content of a large database of notes, along with your personal knowledge.

            We'll start by telling you the question you'll be answering, and feeding you a subset of notes that have been selected from the datbase based on their embedding distance from the question. Then we'll repeat the question, and ask for your response. Notes will be given to you in RoamResearch Markdown format. In RoamResearch Markdown format, references to individual blocks are enclosed in double parentheses, and references to page titles are enclosed in double square brackets.

            To help you answer questions, we've put a link to each page at the top of the page, and a link to each block at the end of each bullet point. Remember these IDs, as you'll be asked to cite them in your answer. Here's an example of the format you should expect:

            ```
            [[Page Title 1]]
            - This is text in a root-level bullet point.[¹](((BlockId1))) 
                - This is text, referencing the [[Page Title 2]], in a child-level bullet point.[*](((BlockId2)))
                    - This is [a link]([[Page Title 3]]) in a child-level bullet point.[²](((BlockId2)))
            [[Page Title 2]]
            - This is some more text in a root-level bullet point.[³](((BlockId3))) 
            ```

            This is the question you'll be answering: 
        "},
    ));
    prompt.push((Role::User, question.to_string()));
    prompt.push((
        Role::System,
        "Here are some notes that might help you answer the question:".to_string(),
    ));
    prompt.push((
        Role::User,
        format_results(conn, results)
            .await
            .wrap_err("Failed to format search results for prompt")?,
    ));
    prompt.push((
        Role::System,
        "Here's the question again, for your reference:".to_string(),
    ));
    prompt.push((Role::User, question.to_string()));
    prompt.push((
        Role::System,
        formatdoc! {"
        Answer the question below in RoamResearch Markdown format:

        - To add a footnote referencing a BlockId: [¹](((BlockId)))
        - To link text to a BlockId: [some inline text](((BlockId)))
        - To link to a page by its title: [[Page Title]]
        - To link text to a page: [some inline text]([[Page Title]])

        Only make links to a [[Page Title]] or to a ((BlockId)). Do not link to anything else.
        
        Be concise in your answer.
    "},
    ));

    // Build the OpenAI request.
    let chat_completion_request = async_openai::types::CreateChatCompletionRequest {
        model: CHAT_MODEL.to_string(),
        messages: prompt
            .into_iter()
            .map(
                |(role, content)| async_openai::types::ChatCompletionRequestMessage {
                    role,
                    content: Some(content),
                    ..Default::default()
                },
            )
            .collect(),
        ..Default::default()
    };

    let answer = openai_client
        .chat()
        .create(chat_completion_request)
        .await
        .wrap_err("Failed to generate answer from OpenAI")?;

    let response_message = answer
        .choices
        .into_iter()
        .map(|choice| choice.message.content)
        .next()
        .flatten()
        .wrap_err("OpenAI responded, but did not include a response message.")?;

    Ok(response_message)
}

pub async fn format_results(conn: &mut SqliteConnection, results: &ResultForest) -> Result<String> {
    let subset_page_list = results
        .get_subset_page_list(conn)
        .wrap_err("Failed to get result subset forest")?;

    let mut out = String::new();

    for subset_page in subset_page_list {
        format_result_page(&mut out, conn, &subset_page)
            .await
            .wrap_err_with(|| format!("Failed to format result page: {}", subset_page.title))?;
    }

    Ok(out)
}

pub async fn format_result_page(
    out: &mut String,
    conn: &mut SqliteConnection,
    results: &result_forest::SubsetPage,
) -> Result<()> {
    // Format the title
    out.push_str(&format!("[[{}]]", results.title));

    // Add the page's subset children.
    for child in &results.children {
        out.push('\n');
        format_result_item(out, conn, child, 0)?;
    }

    Ok(())
}

pub fn format_result_item(
    out: &mut String,
    conn: &mut SqliteConnection,
    item: &result_forest::SubsetItem,
    indent: usize,
) -> Result<()> {
    // Fetch the item from the database.
    let item_db = schema::roam_item::table
        .find(item.id)
        .first::<db::RoamItem>(conn)
        .wrap_err("Failed to get item while formatting prompt")?;

    // Format the bullet
    out.push_str(&"\t".repeat(indent));
    out.push_str(&format!("- {} [*]((({})))", item_db.contents, item.id));

    // Add the item's subset children.
    for child in &item.children {
        out.push('\n');
        format_result_item(out, conn, child, indent + 1)?;
    }

    Ok(())
}
