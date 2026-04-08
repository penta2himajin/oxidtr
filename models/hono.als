-- hono.als
-- Domain model for Hono (TypeScript edge web framework).
-- Language-specific benchmark: TS generics, union types, discriminated unions.

sig Hono {
  router:   one Router,
  basePath: one Str
}

sig Router {
  routes: seq Route
}

sig Route {
  method:  one Str,
  path:    one Str
}

sig Context {
  req:    one HonoRequest,
  status: one Int
}

sig HonoRequest {
  url:    one Str,
  method: one Str,
  path:   one Str
}

abstract sig HTTPMethod {}
one sig GET     extends HTTPMethod {}
one sig POST    extends HTTPMethod {}
one sig PUT     extends HTTPMethod {}
one sig DELETE  extends HTTPMethod {}
one sig PATCH   extends HTTPMethod {}
one sig HEAD    extends HTTPMethod {}
one sig OPTIONS extends HTTPMethod {}

sig Env {
  bindings:  lone Str,
  variables: lone Str
}

sig MiddlewareEntry {
  path: one Str
}
