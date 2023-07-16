use crate::{roam, schema};
use diesel::prelude::*;
use eyre::{Result, WrapErr};
use tracing::instrument;

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = schema::roam_page)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct RoamPage {
    title: String,
    create_time: Option<i64>,
    edit_time: i64,
}

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = schema::roam_item)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct RoamItem {
    id: String,
    parent_page_id: Option<String>,
    parent_item_id: Option<String>,
    contents: String,
    create_time: Option<i64>,
    edit_time: Option<i64>,
}

/// Load a page into the database. Returns the number of items inserted.
#[instrument(level="trace", skip_all, fields(title=page.title))]
pub fn insert_roam_page(conn: &mut SqliteConnection, page: &roam::Page) -> Result<usize> {
    // Create a RoamPage from the roam::Page
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
    for child in &page.children {
        let db_child = RoamItem {
            id: child.uid.clone(),
            parent_page_id: Some(page.title.clone()),
            parent_item_id: None,
            contents: child.string.clone(),
            create_time: child
                .create_time
                .map(|i| {
                    i.try_into()
                        .wrap_err("Failed to convert create time to i64")
                })
                .transpose()?,
            edit_time: child
                .edit_time
                .map(|i| i.try_into().wrap_err("Failed to convert edit time to i64"))
                .transpose()?,
        };

        diesel::insert_into(schema::roam_item::table)
            .values(&db_child)
            .on_conflict(schema::roam_item::id)
            .do_update()
            .set(&db_child)
            .execute(conn)
            .wrap_err_with(|| format!("Failed to insert child: {:#?}", db_child))?;
        item_count += 1;

        item_count += insert_item_children(conn, &child)
            .wrap_err_with(|| format!("Failed to insert child of page '{}'", page.title))?;
    }

    Ok(item_count)
}

/// Loads an item, and all its children, into the database. Returns the number of items
/// inserted.
#[instrument(level = "trace", skip_all, fields(id=parent.uid, contents=parent.string))]
fn insert_item_children(conn: &mut SqliteConnection, parent: &roam::Item) -> Result<usize> {
    let parent_item_id = parent.uid.clone();

    let mut item_count: usize = 0;

    for child in &parent.children {
        // Create the child item.
        let db_item = RoamItem {
            id: child.uid.clone(),
            parent_page_id: None,
            parent_item_id: Some(parent_item_id.clone()),
            contents: child.string.clone(),
            create_time: child
                .create_time
                .map(|i| {
                    i.try_into()
                        .wrap_err("Failed to convert create time to i64")
                })
                .transpose()?,
            edit_time: child
                .edit_time
                .map(|i| i.try_into().wrap_err("Failed to convert edit time to i64"))
                .transpose()?,
        };

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
