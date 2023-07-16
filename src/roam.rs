//! Utilities to parse data from RoamResearch.
//!
//! Derived in part from David Bieber's post, [Roam's JSON Format](https://davidbieber.com/snippets/2020-04-25-roam-json-export/)

#[derive(serde::Deserialize)]
#[serde(transparent)]
pub struct Export {
    pub pages: Vec<Page>,
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Page {
    pub title: String,
    pub edit_time: u64,
    #[serde(default)]
    pub children: Vec<Item>,
    #[serde(default)]
    pub create_time: Option<u64>,
    #[serde(default)]
    pub create_email: Option<String>,
    #[serde(default)]
    pub edit_email: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Item {
    pub uid: String,
    pub string: String,

    #[serde(default)]
    pub create_time: Option<u64>,
    #[serde(default)]
    pub edit_time: Option<u64>,
    #[serde(default)]
    pub children: Vec<Item>,
    #[serde(default)]
    pub edit_email: Option<String>,
    #[serde(default)]
    pub create_email: Option<String>,
}
