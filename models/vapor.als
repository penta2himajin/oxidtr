-- vapor.als
-- Domain model for Vapor (Swift web framework).
-- Language-specific benchmark: Swift protocol, struct, enum, async patterns.
-- Enum variant names use PascalCase (matching Swift extractor's capitalize).

sig Application {
  routes:      seq Route,
  middleware:  seq MiddlewareConfig,
  environment: one AppEnvironment
}

abstract sig AppEnvironment {}
one sig Development extends AppEnvironment {}
one sig Production  extends AppEnvironment {}
one sig Testing     extends AppEnvironment {}

sig Route {
  method: one HTTPMethod,
  path:   seq PathComponent
}

abstract sig PathComponent {}
sig Constant  extends PathComponent { value: one Str }
sig Parameter extends PathComponent { name: one Str }
sig Catchall  extends PathComponent {}
sig Anything  extends PathComponent {}

abstract sig HTTPMethod {}
one sig Get    extends HTTPMethod {}
one sig Post   extends HTTPMethod {}
one sig Put    extends HTTPMethod {}
one sig Delete extends HTTPMethod {}
one sig Patch  extends HTTPMethod {}

sig Request {
  method:  one HTTPMethod,
  url:     one URI,
  headers: seq Header,
  body:    lone Body
}

sig Response {
  status:  one HTTPStatus,
  headers: seq Header,
  body:    lone Body
}

sig URI {
  scheme: lone Str,
  host:   lone Str,
  port:   lone Int,
  path:   one Str,
  query:  lone Str
}

sig HTTPStatus {
  code:         one Int,
  reasonPhrase: one Str
}

sig Header {
  name:  one Str,
  value: one Str
}

sig Body {
  data: one Str
}

sig MiddlewareConfig {
  name: one Str
}
