-- spectre.als
-- Domain model for Spectre.Console (C# terminal UI).
-- Language-specific benchmark: C# class hierarchy, interface, nullable, record.

sig Table {
  title:   lone Str,
  columns: seq TableColumn,
  rows:    seq TableRow,
  border:  one TableBorder
}

sig TableColumn {
  header:    one Str,
  width:     lone Int,
  alignment: lone Alignment
}

sig TableRow {
  cells: seq Str
}

abstract sig TableBorder {}
one sig NoneBorder    extends TableBorder {}
one sig AsciiBorder   extends TableBorder {}
one sig RoundedBorder extends TableBorder {}
one sig HeavyBorder   extends TableBorder {}

abstract sig Alignment {}
one sig LeftAlign   extends Alignment {}
one sig CenterAlign extends Alignment {}
one sig RightAlign  extends Alignment {}

sig Panel {
  header:  lone Str,
  content: one Str,
  border:  one TableBorder
}

sig Tree {
  label: one Str,
  nodes: seq TreeNode
}

sig TreeNode {
  label:    one Str,
  children: seq TreeNode
}

sig Markup {
  text: one Str
}

sig Style {
  foreground: lone Color,
  background: lone Color,
  decoration: lone Decoration
}

sig Color {
  r: one Int,
  g: one Int,
  b: one Int
}

abstract sig Decoration {}
one sig Bold          extends Decoration {}
one sig Italic        extends Decoration {}
one sig Underline     extends Decoration {}
one sig Strikethrough extends Decoration {}

sig Rule {
  title:     lone Str,
  alignment: lone Alignment,
  style:     lone Style
}

sig FigletText {
  text:  one Str,
  color: lone Color
}
