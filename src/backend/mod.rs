pub mod rust;
pub mod typescript;
pub mod jvm;
pub mod schema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}
