use crate::{db, roam, schema, search::Distance};
use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection};
use eyre::{Result, WrapErr};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub struct ResultForest {
    pages: BTreeMap<String, ResultPage>,
}

struct ResultPage {
    /// The name of the result page.
    name: String,

    /// The minimum distance of this result page to the query.
    min_distance: Distance,

    /// The items included in the result.
    included_items: BTreeSet<roam::BlockId>,

    /// The similarity distance for each item.
    item_distances: BTreeMap<roam::BlockId, Distance>,
}

pub struct SubsetPage {
    pub title: String,
    pub min_distance: Distance,
    pub children: Vec<SubsetItem>,
}

pub struct SubsetItem {
    pub id: roam::BlockId,
    pub distance: Option<Distance>,
    pub children: Vec<SubsetItem>,
}

impl ResultForest {
    pub fn new() -> Self {
        Self {
            pages: BTreeMap::new(),
        }
    }

    /// Add a result item to the forest.
    pub fn add_item(
        &mut self,
        conn: &mut SqliteConnection,
        item_id: roam::BlockId,
        distance: Distance,
    ) -> Result<()> {
        // Get the ancestor path of the item, including itself.
        let (page, ancestors) = get_ancestor_ids(conn, item_id)
            .wrap_err("Failed to get page ancestors while adding to ResultForest")?;

        // Get the page's result page, or create a new one.
        let page = self
            .pages
            .entry(page.clone())
            .or_insert_with(|| ResultPage {
                min_distance: distance,
                name: page.clone(),
                included_items: BTreeSet::new(),
                item_distances: BTreeMap::new(),
            });

        // Add the item to the result page.
        for ancestor_id in ancestors {
            page.included_items.insert(ancestor_id);
        }

        // Set its distance.
        page.item_distances.insert(item_id, distance);

        // Update the min_distance, if required.
        if distance < page.min_distance {
            page.min_distance = distance;
        }

        Ok(())
    }

    /// Return the subsetted result list, in order of similarity.
    pub fn get_subset_page_list(&self, conn: &mut SqliteConnection) -> Result<Vec<SubsetPage>> {
        // Get a list of pages, sorted in order of increasing distance.
        let mut pages = self.pages.values().collect::<Vec<_>>();
        pages.sort_by_key(|page| page.min_distance);

        // Get the subset for each page.
        let subset_pages = pages
            .into_iter()
            .map(|page| page.get_subset_page(conn))
            .collect::<Result<Vec<_>>>()?;

        Ok(subset_pages)
    }
}

impl Default for ResultForest {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultPage {
    /// Get the result subset for this page.
    pub fn get_subset_page(&self, conn: &mut SqliteConnection) -> Result<SubsetPage> {
        // Get this page's children.
        let children = schema::roam_item::table
            .filter(schema::roam_item::parent_page_id.eq(&self.name))
            .order(schema::roam_item::order_in_parent.asc())
            .load::<db::RoamItem>(conn)
            .expect("Failed to get children from database");

        // Filter children based on presence in the result set.
        let children_in_result = children
            .into_iter()
            .filter(|child| self.included_items.contains(&child.id));

        // Recurse on children.
        let subset_children = children_in_result
            .map(|child| self.get_subset_item(conn, child.id))
            .collect::<Result<Vec<_>>>()?;

        Ok(SubsetPage {
            title: self.name.clone(),
            min_distance: self.min_distance,
            children: subset_children,
        })
    }

    pub fn get_subset_item(
        &self,
        conn: &mut SqliteConnection,
        item: roam::BlockId,
    ) -> Result<SubsetItem> {
        // Get this item's children.
        let children = schema::roam_item::table
            .filter(schema::roam_item::parent_item_id.eq(item))
            .order(schema::roam_item::order_in_parent.asc())
            .load::<db::RoamItem>(conn)
            .expect("Failed to get children from database");

        // Get the item's distance.
        let distance = self.item_distances.get(&item).copied();

        // Filter children based on presence in the result set.
        let children_in_result = children
            .into_iter()
            .filter(|child| self.included_items.contains(&child.id));

        // Recurse on children.
        let subset_children = children_in_result
            .map(|child| self.get_subset_item(conn, child.id))
            .collect::<Result<Vec<_>>>()?;

        Ok(SubsetItem {
            id: item,
            distance,
            children: subset_children,
        })
    }
}

impl SubsetPage {
    pub fn to_roam_text(&self, indent: usize) -> String {
        let mut text = String::new();

        // Add the page's name.
        text.push_str(&"\t".repeat(indent));
        text.push_str(&format!(
            "`{:.3}` **[[{}]]**\n",
            self.min_distance, self.title
        ));

        // Add the page's children.
        for child in &self.children {
            text.push('\n');
            text.push_str(&child.to_roam_text(indent + 1));
        }

        text
    }
}

impl SubsetItem {
    pub fn to_roam_text(&self, indent: usize) -> String {
        let mut text = String::new();

        // Add the item's name.
        text.push_str(&"\t".repeat(indent));
        text.push_str("- ");

        // Add the item's distance, if it has one, and a reference to it.
        if let Some(distance) = self.distance {
            text.push_str(&format!("`{:.3}` (({}))", distance, self.id));
        } else {
            text.push_str(&format!("(({}))", self.id));
        }

        // Add the item's children.
        for child in &self.children {
            text.push('\n');
            text.push_str(&child.to_roam_text(indent + 1));
        }

        text
    }
}

/// Get the path to an item, starting with the name of the page it's located on, and including the
/// contents of each parent item.
pub fn get_ancestor_ids(
    conn: &mut SqliteConnection,
    item: roam::BlockId,
) -> Result<(String, VecDeque<roam::BlockId>)> {
    let mut path = VecDeque::new();

    let mut current = item;
    loop {
        // Get the item from the database.
        let db_item = schema::roam_item::table
            .find(current)
            .first::<db::RoamItem>(conn)
            .wrap_err("Failed to get item from database")?;

        // Prepend its contents.
        path.push_front(db_item.id);

        match (db_item.parent_item_id, db_item.parent_page_id) {
            (Some(it_id), None) => current = it_id,
            (None, Some(page_id)) => {
                // Return the page name separately.
                return Ok((page_id, path));
            }
            (None, None) | (Some(_), Some(_)) => unreachable!(),
        }
    }
}
