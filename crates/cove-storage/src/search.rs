use crate::StorageError;
use cove_core::MailMessage;
use std::path::Path;
use std::sync::Arc;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, Value, STORED, STRING, TEXT};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, Term};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct MailSearchIndex {
    index: Index,
    reader: IndexReader,
    writer: Arc<Mutex<IndexWriter>>,
    id_field: tantivy::schema::Field,
    subject_field: tantivy::schema::Field,
    preview_field: tantivy::schema::Field,
    body_field: tantivy::schema::Field,
    labels_field: tantivy::schema::Field,
}

impl MailSearchIndex {
    pub fn open_or_create(path: &Path) -> Result<Self, StorageError> {
        std::fs::create_dir_all(path)?;

        let schema = Self::schema();
        let index = match Index::open_in_dir(path) {
            Ok(index) => index,
            Err(_) => Index::create_in_dir(path, schema.clone())?,
        };

        let id_field = schema
            .get_field("id")
            .map_err(|err| StorageError::Data(err.to_string()))?;
        let subject_field = schema
            .get_field("subject")
            .map_err(|err| StorageError::Data(err.to_string()))?;
        let preview_field = schema
            .get_field("preview")
            .map_err(|err| StorageError::Data(err.to_string()))?;
        let body_field = schema
            .get_field("body")
            .map_err(|err| StorageError::Data(err.to_string()))?;
        let labels_field = schema
            .get_field("labels")
            .map_err(|err| StorageError::Data(err.to_string()))?;

        let writer = index.writer(30_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            writer: Arc::new(Mutex::new(writer)),
            id_field,
            subject_field,
            preview_field,
            body_field,
            labels_field,
        })
    }

    pub async fn index_message(&self, message: &MailMessage) -> Result<(), StorageError> {
        let id = message.id.to_string();
        let mut writer = self.writer.lock().await;

        writer.delete_term(Term::from_field_text(self.id_field, &id));
        writer.add_document(doc!(
            self.id_field => id,
            self.subject_field => message.subject.clone(),
            self.preview_field => message.preview.clone(),
            self.body_field => message.body_text.clone().unwrap_or_default(),
            self.labels_field => message.labels.join(" "),
        ))?;

        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub async fn index_messages(&self, messages: &[MailMessage]) -> Result<(), StorageError> {
        if messages.is_empty() {
            return Ok(());
        }

        let mut writer = self.writer.lock().await;

        for message in messages {
            let id = message.id.to_string();
            writer.delete_term(Term::from_field_text(self.id_field, &id));
            writer.add_document(doc!(
                self.id_field => id,
                self.subject_field => message.subject.clone(),
                self.preview_field => message.preview.clone(),
                self.body_field => message.body_text.clone().unwrap_or_default(),
                self.labels_field => message.labels.join(" "),
            ))?;
        }

        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&self, query_text: &str, limit: usize) -> Result<Vec<String>, StorageError> {
        if query_text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(
            &self.index,
            vec![
                self.subject_field,
                self.preview_field,
                self.body_field,
                self.labels_field,
            ],
        );

        let query = parser
            .parse_query(query_text)
            .map_err(|err| StorageError::Data(err.to_string()))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|err| StorageError::Data(err.to_string()))?;

        let mut ids = Vec::with_capacity(top_docs.len());
        for (_score, addr) in top_docs {
            let doc = searcher
                .doc::<tantivy::schema::TantivyDocument>(addr)
                .map_err(|err| StorageError::Data(err.to_string()))?;
            if let Some(value) = doc.get_first(self.id_field) {
                if let Some(text) = value.as_str() {
                    ids.push(text.to_string());
                }
            }
        }

        Ok(ids)
    }

    fn schema() -> Schema {
        let mut builder = Schema::builder();
        builder.add_text_field("id", STRING | STORED);
        builder.add_text_field("subject", TEXT | STORED);
        builder.add_text_field("preview", TEXT | STORED);
        builder.add_text_field("body", TEXT);
        builder.add_text_field("labels", TEXT);
        builder.build()
    }
}
