// Based on Spectre.Console (C# terminal UI library).
// Hand-written C# exercising: class hierarchy, interface pattern,
// nullable reference types, enum, properties.

using System.Collections.Generic;

public class Table
{
    public string? Title { get; set; }
    public List<TableColumn> Columns { get; set; }
    public List<TableRow> Rows { get; set; }
    public TableBorder Border { get; set; }
}

public class TableColumn
{
    public string Header { get; set; }
    public int? Width { get; set; }
    public Alignment? Alignment { get; set; }
}

public class TableRow
{
    public List<string> Cells { get; set; }
}

public abstract class TableBorder {}
public class NoneBorder : TableBorder {}
public class AsciiBorder : TableBorder {}
public class RoundedBorder : TableBorder {}
public class HeavyBorder : TableBorder {}

public enum Alignment
{
    LeftAlign,
    CenterAlign,
    RightAlign
}

public class Panel
{
    public string? Header { get; set; }
    public string Content { get; set; }
    public TableBorder Border { get; set; }
}

public class Tree
{
    public string Label { get; set; }
    public List<TreeNode> Nodes { get; set; }
}

public class TreeNode
{
    public string Label { get; set; }
    public List<TreeNode> Children { get; set; }
}

public class Markup
{
    public string Text { get; set; }
}

public class Style
{
    public Color? Foreground { get; set; }
    public Color? Background { get; set; }
    public Decoration? Decoration { get; set; }
}

public class Color
{
    public int R { get; set; }
    public int G { get; set; }
    public int B { get; set; }
}

public enum Decoration
{
    Bold,
    Italic,
    Underline,
    Strikethrough
}

public class Rule
{
    public string? Title { get; set; }
    public Alignment? Alignment { get; set; }
    public Style? Style { get; set; }
}

public class FigletText
{
    public string Text { get; set; }
    public Color? Color { get; set; }
}
