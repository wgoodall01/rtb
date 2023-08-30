use std::collections::BinaryHeap;

use diesel::{RunQueryDsl, SqliteConnection};
use eyre::{bail, ensure, Context, Result};
use ndarray::{ArrayView, Ix1};
use ordered_float::NotNan;
use tracing::{info_span, instrument};

use crate::{db, embeddings::Embedding, roam, schema};

pub struct SimilaritySearch {
    query: Embedding,
    top_k: usize,

    distance_metric: fn(&Embedding, &Embedding) -> Distance,
}

impl SimilaritySearch {
    pub fn new(query: Embedding) -> SimilaritySearch {
        SimilaritySearch {
            query,
            top_k: 32,
            distance_metric: cosine_distance,
        }
    }

    pub fn with_top_k(self, top_k: usize) -> SimilaritySearch {
        SimilaritySearch { top_k, ..self }
    }

    /// Use a particular distance metric, any ((Embedding, Embedding) -> Distance) function.
    pub fn with_distance_metric(
        self,
        distance_metric: fn(&Embedding, &Embedding) -> Distance,
    ) -> SimilaritySearch {
        SimilaritySearch {
            distance_metric,
            ..self
        }
    }

    /// Execute the similarity query, returning a list of block IDs and associated distance
    /// metrics.
    #[instrument(skip_all)]
    pub async fn execute(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<(Distance, roam::BlockId)>> {
        // Load all the item embeddings.
        let item_embeddings = {
            let span = info_span!("Load item embeddings");
            let _guard = span.enter();

            schema::item_embedding::table
                .load::<db::ItemEmbedding>(conn)
                .wrap_err("Failed to load all item embeddings")?
        };

        ensure!(
            !item_embeddings.is_empty(),
            "No item embeddings found in database"
        );

        // Get the K-most-similar items.
        let k_most_similar: Vec<_> = {
            let span = info_span!("k-NN");
            let _guard = span.enter();

            // The [std::collections::BinaryHeap] is a max-heap, so calling `.pop()` removes the
            // largest item.
            let mut heap = BinaryHeap::new();
            for item_embedding in item_embeddings {
                let distance = (self.distance_metric)(&self.query, &item_embedding.embedding);
                heap.push((distance, item_embedding.item_id));
                if heap.len() > self.top_k {
                    heap.pop();
                }
            }

            heap.into_sorted_vec()
        };

        Ok(k_most_similar)
    }
}

/// Similarity metric, bounded from zero to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, derive_more::Display)]
pub struct Distance(NotNan<f32>);

impl TryFrom<f32> for Distance {
    type Error = eyre::Report;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        if value < 0.0 {
            bail!("Distance metric must be non-negative, got: {}", value);
        }

        let val = NotNan::new(value).wrap_err("Distance cannot be NaN")?;

        Ok(Distance(val))
    }
}

impl From<Distance> for f32 {
    fn from(distance: Distance) -> Self {
        distance.0.into_inner()
    }
}

/// Compute a cosine distance metric between two embeddings.
///
/// This metric is normalized to [0, 1], where 0 is most similar, and 1 is least similar.
pub fn cosine_distance(a: &Embedding, b: &Embedding) -> Distance {
    let a: ArrayView<f32, Ix1> = a.view();
    let b: ArrayView<f32, Ix1> = b.view();

    let norm_a = a.dot(&a).sqrt();
    let norm_b = b.dot(&b).sqrt();

    let similarity = a.dot(&b) / (norm_a * norm_b);

    (1.0 - similarity)
        .try_into()
        .expect("Cosine distance was out of range.")
}

/// Compute Euclidean distance between two embeddings.
pub fn euclidean_distance(a: &Embedding, b: &Embedding) -> Distance {
    let a: ArrayView<f32, Ix1> = a.view();
    let b: ArrayView<f32, Ix1> = b.view();

    let sub = &a - &b;
    let sum_squares = sub.dot(&sub);
    let distance = sum_squares.sqrt();

    distance
        .try_into()
        .expect("Euclidean distance was out of range")
}
