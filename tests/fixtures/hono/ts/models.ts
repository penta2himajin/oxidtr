// Based on Hono's core type definitions.
// Hand-written TypeScript exercising: generics, union types,
// string literal unions, interface inheritance.

export interface Hono {
  router: Router;
  basePath: string;
}

export interface Router {
  routes: Route[];
}

export interface Route {
  method: string;
  path: string;
}

export interface Context {
  req: HonoRequest;
  status: number;
}

export interface HonoRequest {
  url: string;
  method: string;
  path: string;
}

export type HTTPMethod =
  | 'GET'
  | 'POST'
  | 'PUT'
  | 'DELETE'
  | 'PATCH'
  | 'HEAD'
  | 'OPTIONS';

export interface Env {
  bindings?: string;
  variables?: string;
}

export interface MiddlewareEntry {
  path: string;
}
