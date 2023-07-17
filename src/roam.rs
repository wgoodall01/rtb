//! Utilities to parse data from RoamResearch.
//!
//! Derived in part from David Bieber's post, [Roam's JSON Format](https://davidbieber.com/snippets/2020-04-25-roam-json-export/)

use std::fmt;
use std::str::FromStr;

use diesel::{backend::Backend, deserialize, serialize, sql_types, sqlite::Sqlite};
use eyre::{bail, Report, WrapErr};

/// A Roam block identifier.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Hash,
    PartialOrd,
    Ord,
    diesel::AsExpression,
    diesel::FromSqlRow,
)]
#[diesel(sql_type = sql_types::Text)]
pub struct BlockId([u8; 9]);

impl FromStr for BlockId {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 9 {
            bail!("Roam BlockId must be 9 characters long");
        }

        let mut bytes = [0; 9];
        for (i, c) in s.chars().enumerate() {
            bytes[i] = c.try_into().wrap_err("Failed to convert byte of BlockId")?;
        }

        Ok(BlockId(bytes))
    }
}

impl From<BlockId> for String {
    fn from(id: BlockId) -> Self {
        id.as_ref().to_owned()
    }
}

impl AsRef<str> for BlockId {
    fn as_ref(&self) -> &str {
        std::str::from_utf8(&self.0).expect("invalid internal state: BlockId is not valid UTF-8")
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl serialize::ToSql<sql_types::Text, Sqlite> for BlockId {
    fn to_sql<'b>(&'b self, out: &mut serialize::Output<'b, '_, Sqlite>) -> serialize::Result {
        let id_str = self.as_ref();
        <str as serialize::ToSql<sql_types::Text, Sqlite>>::to_sql(id_str, out)
    }
}

impl deserialize::FromSql<sql_types::Text, Sqlite> for BlockId {
    fn from_sql(raw: <Sqlite as Backend>::RawValue<'_>) -> deserialize::Result<Self> {
        let id_str = <String as deserialize::FromSql<sql_types::Text, Sqlite>>::from_sql(raw)?;
        id_str.parse().map_err(Into::into)
    }
}

impl serde::Serialize for BlockId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_ref())
    }
}

impl<'de> serde::Deserialize<'de> for BlockId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id_str = String::deserialize(deserializer)?;
        id_str.parse().map_err(serde::de::Error::custom)
    }
}

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
    pub uid: BlockId,
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
