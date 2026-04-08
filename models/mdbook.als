-- mdbook.als
-- Domain model for mdBook (Rust documentation tool).
-- Language-specific benchmark: Rust enum variants, Option, Vec, trait impls.
-- Field names use snake_case to match Rust conventions.

sig Book {
  sections: seq BookItem
}

abstract sig BookItem {}

sig Chapter extends BookItem {
  name:         one Str,
  content:      one Str,
  number:       lone SectionNumber,
  sub_items:    seq BookItem,
  path:         lone Str,
  source_path:  lone Str,
  parent_names: seq Str
}

sig Separator extends BookItem {}

sig PartTitle extends BookItem {
  title: one Str
}

sig SectionNumber {}

sig Summary {
  title:              lone Str,
  prefix_chapters:    seq SummaryItem,
  numbered_chapters:  seq SummaryItem,
  suffix_chapters:    seq SummaryItem
}

abstract sig SummaryItem {}

sig Link extends SummaryItem {
  name:         one Str,
  location:     lone Str,
  number:       lone SectionNumber,
  nested_items: seq SummaryItem
}

-- Note: Separator and PartTitle variants are shared between BookItem and SummaryItem enums.
-- In Rust, both enums have `Separator` and `PartTitle(String)` variants.
-- Alloy requires unique sig names, so BookItem's versions are used (declared above).
-- The Rust extractor maps both to the same name; the differ matches them to the BookItem sigs.

sig BookConfig {
  title:       lone Str,
  authors:     seq Str,
  description: lone Str,
  src:         one Str,
  language:    lone Str
}
