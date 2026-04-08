// Based on mdBook's book/summary types.
// Hand-written Rust exercising: enum (tuple+struct+unit variants),
// Option, Vec, newtype struct, trait derives.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Book {
    pub sections: Vec<BookItem>,
}

#[derive(Debug, Clone)]
pub enum BookItem {
    Chapter(Chapter),
    Separator,
    PartTitle(String),
}

#[derive(Debug, Clone)]
pub struct Chapter {
    pub name: String,
    pub content: String,
    pub number: Option<SectionNumber>,
    pub sub_items: Vec<BookItem>,
    pub path: Option<PathBuf>,
    pub source_path: Option<PathBuf>,
    pub parent_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionNumber(pub Vec<u32>);

#[derive(Debug, Clone)]
pub struct Summary {
    pub title: Option<String>,
    pub prefix_chapters: Vec<SummaryItem>,
    pub numbered_chapters: Vec<SummaryItem>,
    pub suffix_chapters: Vec<SummaryItem>,
}

#[derive(Debug, Clone)]
pub enum SummaryItem {
    Link(Link),
    Separator,
    PartTitle(String),
}

#[derive(Debug, Clone)]
pub struct Link {
    pub name: String,
    pub location: Option<PathBuf>,
    pub number: Option<SectionNumber>,
    pub nested_items: Vec<SummaryItem>,
}

#[derive(Debug, Clone)]
pub struct BookConfig {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub src: PathBuf,
    pub language: Option<String>,
}
