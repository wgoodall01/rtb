use diesel::{backend::Backend, deserialize, serialize, sql_types, sqlite::Sqlite};
use eyre::{eyre, Result, WrapErr};
use ndarray::{Array, ArrayView, Ix1};
use ordered_float::NotNan;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, PartialEq, Clone, Deserialize, Serialize, diesel::AsExpression, diesel::FromSqlRow,
)]
#[diesel(sql_type = sql_types::Blob)]
pub struct Embedding(Array<f32, Ix1>);

impl Embedding {
    pub fn from_bytes(bytes: &[u8]) -> Embedding {
        let floats = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Embedding(floats)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.0
            .iter()
            .flat_map(|f| f.to_le_bytes().into_iter())
            .collect()
    }

    pub fn dimensionality(&self) -> usize {
        self.0.len()
    }

    pub fn view(&self) -> ArrayView<f32, Ix1> {
        self.0.view()
    }
}

impl AsRef<[f32]> for Embedding {
    fn as_ref(&self) -> &[f32] {
        self.0
            .as_slice()
            .expect("Embedding is not contiguous in memory")
    }
}

impl serialize::ToSql<sql_types::Blob, Sqlite> for Embedding
where
    Vec<u8>: serialize::ToSql<sql_types::Blob, Sqlite>,
{
    fn to_sql<'b>(&'b self, out: &mut serialize::Output<'b, '_, Sqlite>) -> serialize::Result {
        let bytes = self.to_bytes();
        out.set_value(bytes);
        Ok(diesel::serialize::IsNull::No)
    }
}

impl deserialize::FromSql<sql_types::Blob, Sqlite> for Embedding {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> deserialize::Result<Self> {
        <Vec<u8> as deserialize::FromSql<sql_types::Blob, Sqlite>>::from_sql(bytes)
            .map(|vec| Embedding::from_bytes(&vec))
    }
}

/// Compute a batch of embeddings.
pub async fn embed_text_batch(
    openai: &async_openai::Client<async_openai::config::OpenAIConfig>,
    sources: &[&str],
) -> Result<Vec<Embedding>> {
    use async_openai::types::{CreateEmbeddingRequest, EmbeddingInput};

    // Check that none of the strings are empty (this makes the API unhappy).
    if sources.iter().any(|s| s.is_empty()) {
        return Err(eyre!("Cannot create embedding for empty string."));
    }

    // Create the embedding request.
    let request = CreateEmbeddingRequest {
        model: "text-embedding-ada-002".to_string(),
        input: EmbeddingInput::StringArray(sources.iter().map(|&s| s.to_string()).collect()),
        user: Some("rtb".to_string()),
    };

    // Send the embedding request.
    let response = openai
        .embeddings()
        .create(request)
        .await
        .wrap_err("Failed to create embeddings")?;

    // Return the embeddings.
    let embeddings = response
        .data
        .into_iter()
        .map(|e| e.embedding)
        .map(|e| Embedding(e.into()))
        .collect();

    Ok(embeddings)
}

/// Compute a single embedding.
pub async fn embed_text(
    openai: &async_openai::Client<async_openai::config::OpenAIConfig>,
    source: &str,
) -> Result<Embedding> {
    let embeddings = embed_text_batch(openai, &[source]).await?;
    Ok(embeddings.into_iter().next().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_embedding_to_bytes() {
        let embedding = Embedding(ndarray::array![1.0, 2.0, 3.0]);
        let bytes = embedding.to_bytes();
        let embedding2 = Embedding::from_bytes(&bytes);
        assert_eq!(embedding, embedding2);
    }
}
