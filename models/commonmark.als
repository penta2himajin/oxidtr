-- commonmark.als
-- CommonMark AST domain model.
-- Reference: https://spec.commonmark.org/
--
-- Purpose: benchmark for hand-written extraction accuracy.
-- Each target language has a well-known OSS implementation
-- that can be checked against this model.

-------------------------------------------------------------------------------
-- Document root
-------------------------------------------------------------------------------

sig Document {
  blocks: seq Block
}

-------------------------------------------------------------------------------
-- Block nodes
-------------------------------------------------------------------------------

abstract sig Block {}

sig Heading extends Block {
  level:   one Int,
  inlines: seq Inline
}

sig Paragraph extends Block {
  inlines: seq Inline
}

sig BlockQuote extends Block {
  items: seq Block
}

sig CodeBlock extends Block {
  info:    lone Str,
  literal: one Str
}

sig HtmlBlock extends Block {
  literal: one Str
}

sig ThematicBreak extends Block {}

sig ListBlock extends Block {
  ordered: one Bool,
  tight:   one Bool,
  start:   lone Int,
  items:   seq ListItem
}

sig ListItem extends Block {
  contents: seq Block
}

-------------------------------------------------------------------------------
-- Inline nodes
-------------------------------------------------------------------------------

abstract sig Inline {}

sig Text extends Inline {
  literal: one Str
}

sig CodeSpan extends Inline {
  literal: one Str
}

sig Emphasis extends Inline {
  children: seq Inline
}

sig Strong extends Inline {
  children: seq Inline
}

sig Link extends Inline {
  destination: one Str,
  title:       lone Str,
  children:    set Inline
}

sig Image extends Inline {
  destination: one Str,
  title:       lone Str,
  children:    set Inline
}

sig HtmlInline extends Inline {
  literal: one Str
}

sig SoftBreak extends Inline {}

sig LineBreak extends Inline {}

-------------------------------------------------------------------------------
-- Structural constraints
-------------------------------------------------------------------------------

fact HeadingLevelBound {
  all h: Heading | h.level >= 1 and h.level <= 6
}

fact NoCyclicBlocks {
  no b: BlockQuote | b in b.^items
}

fact ListItemNoCycle {
  no li: ListItem | some b: li.contents | b in ListItem and b = li
}

-------------------------------------------------------------------------------
-- Predicates
-------------------------------------------------------------------------------

pred addBlock[doc: one Document, b: one Block] {
  b in doc.blocks
}

pred createHeading[h: one Heading, lvl: one Int] {
  h.level = lvl
}

pred createLink[l: one Link, dest: one Str] {
  l.destination = dest
}

-------------------------------------------------------------------------------
-- Assertions
-------------------------------------------------------------------------------

assert HeadingLevelValid {
  all h: Heading | h.level >= 1 and h.level <= 6
}
