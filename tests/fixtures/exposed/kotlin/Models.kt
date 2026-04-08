// Based on JetBrains Exposed SQL DSL.
// Hand-written Kotlin exercising: object declarations, sealed class,
// data class, generics, extension-style patterns.

data class Table(
    val tableName: String,
    val columns: List<Column>,
    val primaryKey: PrimaryKey?
)

data class Column(
    val name: String,
    val table: Table,
    val columnType: ColumnType,
    val nullable: Boolean
)

data class ColumnType(
    val sqlType: String
)

data class PrimaryKey(
    val columns: List<Column>
)

sealed class Op

data class EqOp(
    val left: Column,
    val right: Expr
) : Op()

data class NeqOp(
    val left: Column,
    val right: Expr
) : Op()

data class AndOp(
    val conditions: List<Op>
) : Op()

data class OrOp(
    val conditions: List<Op>
) : Op()

data class LikeOp(
    val column: Column,
    val pattern: String
) : Op()

data class IsNullOp(
    val column: Column
) : Op()

sealed class Expr

data class LiteralExpr(
    val value: String
) : Expr()

data class ColumnExpr(
    val column: Column
) : Expr()

data class Query(
    val table: Table,
    val where: Op?,
    val limit: Int?
)

data class Transaction(
    val queries: List<Query>
)
