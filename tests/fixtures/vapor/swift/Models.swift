// Based on Vapor (Swift web framework).
// Hand-written Swift exercising: enum with associated values,
// struct, protocol-style patterns, optional chaining.

struct Application {
    let routes: [Route]
    let middleware: [MiddlewareConfig]
    let environment: AppEnvironment
}

enum AppEnvironment {
    case development
    case production
    case testing
}

struct Route {
    let method: HTTPMethod
    let path: [PathComponent]
}

enum PathComponent {
    case constant(value: String)
    case parameter(name: String)
    case catchall
    case anything
}

enum HTTPMethod {
    case get
    case post
    case put
    case delete
    case patch
}

struct Request {
    let method: HTTPMethod
    let url: URI
    let headers: [Header]
    let body: Body?
}

struct Response {
    let status: HTTPStatus
    let headers: [Header]
    let body: Body?
}

struct URI {
    let scheme: String?
    let host: String?
    let port: Int?
    let path: String
    let query: String?
}

struct HTTPStatus {
    let code: Int
    let reasonPhrase: String
}

struct Header {
    let name: String
    let value: String
}

struct Body {
    let data: String
}

struct MiddlewareConfig {
    let name: String
}
