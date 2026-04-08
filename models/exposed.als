-- exposed.als
-- Domain model for Exposed (Kotlin SQL DSL by JetBrains).
-- Language-specific benchmark: Kotlin object declaration, sealed class, generics.

sig Table {
  tableName:  one Str,
  columns:    seq Column,
  primaryKey: lone PrimaryKey
}

sig Column {
  name:       one Str,
  table:      one Table,
  columnType: one ColumnType,
  nullable:   one Bool
}

sig ColumnType {
  sqlType: one Str
}

sig PrimaryKey {
  columns: seq Column
}

abstract sig Op {}

sig EqOp extends Op {
  left:  one Column,
  right: one Expr
}

sig NeqOp extends Op {
  left:  one Column,
  right: one Expr
}

sig AndOp extends Op {
  conditions: seq Op
}

sig OrOp extends Op {
  conditions: seq Op
}

sig LikeOp extends Op {
  column:  one Column,
  pattern: one Str
}

sig IsNullOp extends Op {
  column: one Column
}

abstract sig Expr {}

sig LiteralExpr extends Expr {
  value: one Str
}

sig ColumnExpr extends Expr {
  column: one Column
}

sig Query {
  table: one Table,
  where: lone Op,
  limit: lone Int
}

sig Transaction {
  queries: seq Query
}
