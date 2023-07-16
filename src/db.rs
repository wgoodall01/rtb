use crate::{roam, schema};
use async_recursion::async_recursion;
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

#[instrument(level="trace", skip_all, fields(title=page.title))]
pub async fn insert_roam_page(conn: &mut SqliteConnection, page: &roam::Page) -> Result<()> {
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

        insert_item_children(conn, &child)
            .await
            .wrap_err_with(|| format!("Failed to insert child of page '{}'", page.title))?;
    }

    Ok(())
}

#[async_recursion]
#[instrument(level = "trace", skip_all, fields(id=parent.uid, contents=parent.string))]
async fn insert_item_children(conn: &mut SqliteConnection, parent: &roam::Item) -> Result<()> {
    let parent_item_id = parent.uid.clone();

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

        // Insert the child item's children.
        insert_item_children(conn, child)
            .await
            .wrap_err_with(|| format!("Failed to insert child of item '{}'", parent.uid))?;
    }

    Ok(())
}
