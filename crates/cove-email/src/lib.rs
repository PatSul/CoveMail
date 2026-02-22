mod backend;
mod error;
mod service;

pub use backend::{
    default_protocol_for_provider, EmailBackend, EwsBackend, FetchResult, ImapSmtpBackend,
    JmapBackend, OutgoingAttachment, OutgoingMail, ProtocolSettings,
};
pub use error::EmailError;
pub use service::EmailService;
