use std::collections::VecDeque;

use crate::{embeddings, roam, schema};
use diesel::prelude::*;
use eyre::{Result, WrapErr};
use tracing::instrument;

/// If a block references this page, that block and its children will not be imported.
pub const EXCLUDE_PAGE: &str = "Roam Third Brain/Exclude";

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = schema::roam_page)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct RoamPage {
    pub title: String,
    pub create_time: Option<i64>,
    pub edit_time: i64,
}

impl RoamPage {
    fn try_from_roam_json(page: &roam::Page) -> Result<RoamPage> {
        let db_page = RoamPage {
            title: page.title.clone(),
            edit_time: page
                .edit_time
                .try_into()
                .wrap_err("Failed to convert edit time to i64")?,
            create_time: page
                .create_time
                .map(|i| {
                    i.try_into()
                        .wrap_err("Failed to convert create time to i64")
                })
                .transpose()?,
        };

        Ok(db_page)
    }
}

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = schema::roam_item)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct RoamItem {
    pub id: roam::BlockId,
    pub parent_page_id: Option<String>,
    pub parent_item_id: Option<roam::BlockId>,
    pub order_in_parent: i32,
    pub contents: String,
    pub create_time: Option<i64>,
    pub edit_time: Option<i64>,
}

impl RoamItem {
    pub fn try_from_roam_json_root(
        page_title: &str,
        item: &roam::Item,
        order: u64,
    ) -> Result<RoamItem> {
        let db_item = RoamItem {
            id: item.uid,
            parent_page_id: Some(page_title.to_owned()),
            parent_item_id: None,
            order_in_parent: order
                .try_into()
                .wrap_err("Order out of range for db integer")?,
            contents: item.string.clone(),
            create_time: item
                .create_time
                .map(|i| {
                    i.try_into()
                        .wrap_err("Failed to convert create time to i64")
                })
                .transpose()?,
            edit_time: item
                .edit_time
                .map(|i| i.try_into().wrap_err("Failed to convert edit time to i64"))
                .transpose()?,
        };

        Ok(db_item)
    }

    pub fn try_from_roam_json_child(
        parent_id: roam::BlockId,
        item: &roam::Item,
        order: u64,
    ) -> Result<RoamItem> {
        let db_item = RoamItem {
            id: item.uid,
            parent_page_id: None,
            parent_item_id: Some(parent_id),
            order_in_parent: order
                .try_into()
                .wrap_err("Order out of range for db integer")?,
            contents: item.string.clone(),
            create_time: item
                .create_time
                .map(|i| {
                    i.try_into()
                        .wrap_err("Failed to convert create time to i64")
                })
                .transpose()?,
            edit_time: item
                .edit_time
                .map(|i| i.try_into().wrap_err("Failed to convert edit time to i64"))
                .transpose()?,
        };

        Ok(db_item)
    }
}

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = schema::item_embedding)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ItemEmbedding {
    pub item_id: roam::BlockId,
    pub embedded_text: String,
    pub embedding: embeddings::Embedding,
}

/// Whether or not this item and its children should be excluded.
fn should_exclude_subtree(item: &roam::Item) -> bool {
    item.string.contains(&format!("[[{EXCLUDE_PAGE}]]"))
}

/// Delete an item, and the subtree it defines, from the database.
fn delete_item_and_subtree(conn: &mut SqliteConnection, item_id: &roam::BlockId) -> Result<()> {
    diesel::sql_query(
        r"
        -- Rely on the foreign key constraint to delete the item's children.
        delete from roam_item where id = ?;
        ",
    )
    .bind::<diesel::sql_types::Text, _>(item_id.to_string())
    .execute(conn)
    .wrap_err("Failed to delete children")?;

    Ok(())
}

/// Load a page into the database. Returns the number of items inserted.
#[instrument(level="trace", skip_all, fields(title=page.title))]
pub fn insert_roam_page(conn: &mut SqliteConnection, page: &roam::Page) -> Result<usize> {
    // Create a RoamPage from the roam::Page
    let db_page = RoamPage::try_from_roam_json(page)?;

    // Insert the RoamPage
    diesel::insert_into(schema::roam_page::table)
        .values(&db_page)
        .on_conflict(schema::roam_page::title)
        .do_update()
        .set(&db_page)
        .execute(conn)
        .wrap_err_with(|| format!("Failed to insert page: {:?}", page.title))?;

    let mut item_count = 0;

    // Insert its children.
    for (i, child) in page.children.iter().enumerate() {
        if should_exclude_subtree(child) {
            delete_item_and_subtree(conn, &child.uid).context("Failed to delete excluded item")?;
            continue;
        }

        let db_child = RoamItem::try_from_roam_json_root(
            &page.title,
            child,
            i.try_into().wrap_err("Child index out of range")?,
        )?;

        diesel::insert_into(schema::roam_item::table)
            .values(&db_child)
            .on_conflict(schema::roam_item::id)
            .do_update()
            .set(&db_child)
            .execute(conn)
            .wrap_err_with(|| format!("Failed to insert child: {:#?}", db_child))?;
        item_count += 1;

        item_count += insert_item_children(conn, child)
            .wrap_err_with(|| format!("Failed to insert child of page '{}'", page.title))?;
    }

    Ok(item_count)
}

/// Loads an item, and all its children, into the database. Returns the number of items
/// inserted.
#[instrument(level = "trace", skip_all, fields(id=%parent.uid, contents=parent.string))]
fn insert_item_children(conn: &mut SqliteConnection, parent: &roam::Item) -> Result<usize> {
    let parent_item_id = parent.uid;

    let mut item_count: usize = 0;

    for (i, child) in parent.children.iter().enumerate() {
        if should_exclude_subtree(child) {
            delete_item_and_subtree(conn, &child.uid).context("Failed to delete excluded item")?;
            continue;
        }

        // Create the child item.
        let db_item = RoamItem::try_from_roam_json_child(
            parent_item_id,
            child,
            i.try_into().wrap_err("Child index out of range")?,
        )?;

        // Insert the child item, updating all columns on conflict.
        diesel::insert_into(schema::roam_item::table)
            .values(&db_item)
            .on_conflict(schema::roam_item::id)
            .do_update()
            .set(&db_item)
            .execute(conn)
            .wrap_err_with(|| format!("Failed to insert child item: {db_item:?}"))?;
        item_count += 1;

        // Insert the child item's children.
        item_count += insert_item_children(conn, child)
            .wrap_err_with(|| format!("Failed to insert child of item '{}'", parent.uid))?;
    }

    Ok(item_count)
}

/// Get the path to an item, starting with the name of the page it's located on, and including the
/// contents of each parent item.
pub fn get_content_with_ancestors(
    conn: &mut SqliteConnection,
    item: roam::BlockId,
) -> (String, VecDeque<String>) {
    let mut path = VecDeque::new();

    let mut current = item;
    loop {
        // Get the item from the database.
        let db_item = schema::roam_item::table
            .find(current)
            .first::<RoamItem>(conn)
            .expect("Failed to get item from database");

        // Prepend its contents.
        path.push_front(db_item.contents);

        match (db_item.parent_item_id, db_item.parent_page_id) {
            (Some(it_id), None) => current = it_id,
            (None, Some(page_id)) => {
                // Return the page name separately.
                return (page_id, path);
            }
            (None, None) | (Some(_), Some(_)) => unreachable!(),
        }
    }
}

/// Format the ready-to-embed text for an item.
///
/// This will include the item's contents, and the contents of its parent items and page.
pub fn get_embeddable_text(conn: &mut SqliteConnection, item: roam::BlockId) -> Result<String> {
    let mut text = String::new();

    let (title, path) = get_content_with_ancestors(conn, item);
    // Push the page title.
    text.push_str(&format!("# {title}\n\n"));

    // Push each path item with successive indentation.
    for (i, item) in path.into_iter().enumerate() {
        text.push_str(&"\t".repeat(i));
        text.push_str(" - ");
        text.push_str(&item);
        text.push('\n');
    }

    Ok(text)
}
