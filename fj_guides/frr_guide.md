# Flapjack Request Router (FRR) - Detailed Design & Architecture (Golang)

This document outlines the architecture for the Flapjack Request Router (FRR), a Golang-based service designed to be a stateless, high-performance, maintainable, and secure component of the Flapjack system. It serves as a guide for both human engineers and LLM-assisted development.

## 1. Core Principles

*   **Statelessness (as much as possible):** FRR instances should be stateless, deriving necessary routing and authentication/authorization information from a synchronized in-memory cache, which is sourced from the Global Master Database (GMD). This facilitates horizontal scaling and resilience.
*   **High Performance:** Leverage Go's concurrency, efficient networking, and minimize processing overhead per request.
*   **Maintainability & Testability:** Clean code, clear separation of concerns, and a design conducive to Test-Driven Development (TDD).
*   **Minimal & Vetted Dependencies:** Rely on the Go standard library where feasible. Third-party libraries will be carefully chosen for stability, performance, and security.
*   **Security First & Reliability:** Robust authentication and authorization, secure handling of sensitive data, protection against common web vulnerabilities (though DDoS protection is offloaded to upstream components), and mechanisms to ensure data consistency with the GMD.

## 2. High-Level Architectural Diagram

```
External User Request (via Geo-DNS)
        |
        v
+---------------------------------+
| Load Balancer (Regional)        | e.g., AWS ALB/NLB, Cloudflare
| (Handles HTTPS Termination, DDoS)|
+---------------------------------+
        |
        v (HTTP/HTTPS)
+--------------------------------------------------------------------+
| FRRNodeVM                                                          |
| +----------------------------------------------------------------+ |
| | Flapjack Request Router (FRR) Go Application                 | |
| |                                                                | |
| |  [Incoming Request]                                            | |
| |      |                                                         | |
| |      v                                                         | |
| |  +-----------------------+                                     | |
| |  | HTTP Server (e.g. Chi)|                                     | |
| |  +-----------------------+                                     | |
| |      | ^ Middleware Chain                                      | |
| |      | |---------------------> +----------------------------+  | |
| |      |                         | Logging Middleware         |  | |
| |      | <---------------------| +----------------------------+  | |
| |      |                         | Metrics Middleware         |  | |
| |      | <---------------------| +----------------------------+  | |
| |      |                         | Panic Recovery Middleware  |  | |
| |      | <---------------------| +----------------------------+  | |
| |      |                         | API Key Auth Middleware    |  | |
| |      | <---------------------| +----------------------------+  | |
| |      |                         | Authorization Middleware   |  | |
| |      | <---------------------| +----------------------------+  | |
| |      |                         | Routing Logic Middleware   |  | |
| |      | <---------------------| +----------------------------+  | |
| |      |                                                         | |
| |      v                                                         | |
| |  +-----------------------+                                     | |
| |  | Reverse Proxy Handler |                                     | |
| |  | (Path Rewrite, Proxy) | ----> Backend SearchNodeVM (Meilisearch) | |
| |  +-----------------------+                                     | |
| |      |                                                         | |
| |      v (Response to client)                                    | |
| |                                                                | |
| +----------------------------------------------------------------+ |
|          ^                                                       |
|          | FRR Data Sync (Updates Cache)                         |
| +----------------------------------------------------------------+ |
| | In-Memory Cache Store                                          | |
| | (Routing Rules, API Key Details)                               | |
| | (sync.RWMutex protected Go map)                                | |
| +----------------------------------------------------------------+ |
|          ^                                                       |
|          | (Supabase Realtime + Periodic Staleness Check)        |
| +----------------------------------------------------------------+ |
| | FRR Data Sync Service (Goroutine)                              | |
| | (Subscribes to GMD changes, performs heartbeat checks)         | |
| +----------------------------------------------------------------+ |
+--------------------------------------------------------------------+
        |
        v
Global Master Database (Supabase/Postgres)
```

## 3. Key Components & Proposed Go Package Structure

A monolithic repository for the FRR service seems appropriate.

```
flapjack-frr/
├── cmd/frr/                       // Main application entry point
│   └── main.go
├── internal/
│   ├── config/                    // Configuration loading (e.g., from env vars, files)
│   │   ├── config.go
│   │   └── config_test.go
│   ├── server/                    // HTTP server setup, middleware registration, core request lifecycle
│   │   ├── server.go
│   │   ├── server_test.go
│   │   ├── routes.go              // Defines HTTP routes and links to handlers
│   │   └── routes_test.go
│   ├── middleware/                // HTTP middleware implementations
│   │   ├── logger.go              // Request/response logging
│   │   ├── logger_test.go
│   │   ├── metrics.go             // Prometheus metrics collection
│   │   ├── metrics_test.go
│   │   ├── recovery.go            // Panic recovery
│   │   ├── recovery_test.go
│   │   ├── apikey_auth.go         // API Key authentication
│   │   ├── apikey_auth_test.go
│   │   ├── authorization.go       // Permission enforcement based on logical index & action
│   │   ├── authorization_test.go
│   │   └── routing_resolver.go    // Resolves logical index to physical backend details
│   │   └── routing_resolver_test.go
│   ├── cache/                     // In-memory cache for routing, API keys, and permissions
│   │   ├── cache.go               // Cache implementation (e.g., Go map with RWMutex)
│   │   └── cache_test.go
│   ├── datasync/                  // Synchronization service for GMD -> FRR Cache
│   │   ├── gmd_syncer.go          // Logic for Supabase Realtime subscription, cache updates, and staleness checks
│   │   └── gmd_syncer_test.go
│   ├── proxy/                     // Reverse proxy logic to backend search engines
│   │   ├── reverse_proxy.go       // Custom director, response modifier
│   │   └── reverse_proxy_test.go
│   ├── util/                      // Shared utility functions (if any)
│   │   └── util.go
├──pkg/                            // Shared libraries if we were to expose parts of FRR (unlikely for this service)
│   └── errs/                      // Custom error types (optional)
├── go.mod
├── go.sum
├── Dockerfile                     // For containerizing the FRR
├── Makefile                       // For build, test, lint commands
├── .golangci.yml                  // Linter configuration
├── .env.example                   // Example environment variables
├── tests/                         // Integration tests (separate from unit tests in internal/)
│   └── integration_test.go
```

## 4. Dependency Selection & Justification

The goal is to minimize dependencies, favoring the standard library, and choosing well-maintained, secure, and performant libraries where necessary.

*   **HTTP Router:**
    *   **`go-chi/chi` (v5):** Lightweight, idiomatic Go, excellent performance, great middleware support, no reflection. Well-regarded in the Go community.
*   **Logging:**
    *   **`rs/zerolog`:** High-performance, zero-allocation structured JSON logging. Excellent for production environments.
*   **Configuration Management:**
    *   **`spf13/viper`:** Versatile, supports environment variables, config files (JSON, TOML, YAML), and flags.
*   **Metrics (Prometheus):**
    *   **`prometheus/client_golang`:** Official Go client library for Prometheus.
*   **Supabase Client (for FRR Data Sync via Realtime):**
    *   **`nedpals/supabase-go`**: Most active and recommended Go client for Supabase, including Realtime features.
        *   *Critical Evaluation Point:* Reliability of Realtime CDC. The heartbeat mechanism (see `datasync`) is a safeguard.
        *   *Fallback (if Realtime CDC proves consistently problematic beyond the mitigation of the heartbeat check):* A more robust periodic polling of relevant GMD tables might be needed, though this is less ideal than event-driven updates.
*   **UUID Generation (if needed for internal IDs):**
    *   **`google/uuid`:** Robust, well-tested UUID generation. (Note: Most primary keys are from GMD).
*   **API Key Handling:**
    *   **Strategy: Opaque Tokens + Cached Details (from GMD).** FRR receives an opaque token, looks it up in its in-memory cache (sourced from GMD). No JWT parsing library needed within FRR for customer API keys.
*   **Standard Library Usage:**
    *   `net/http` & `net/http/httputil`: Core for HTTP server and reverse proxy.
    *   `context`: For request-scoped data, cancellation.
    *   `sync`: Crucial for `internal/cache/cache.go` (`sync.RWMutex`).
    *   `encoding/json`: For JSON payloads.
    *   `time`: For timeouts, etc.

## 5. Security Considerations for Dependencies

*   Regularly run `go list -m -json all | govulncheck` to scan for vulnerabilities.
*   Keep dependencies updated.
*   Choose libraries with active maintenance and good security track records.

## 6. Detailed Component Breakdown & Responsibilities

*   **`internal/config/config.go`:**
    *   Defines a struct for all FRR configuration (server port, GMD connection URL, log level, timeouts, heartbeat intervals, staleness thresholds).
    *   Loads configuration from environment variables (primary) and optionally a config file using Viper.
    *   Provides type-safe access to configuration values.

*   **`internal/server/server.go` & `routes.go`:**
    *   `server.go`: Initializes the Chi HTTP server, registers global middleware (logger, metrics, recovery), mounts routes from `routes.go`. Handles graceful shutdown.
    *   `routes.go`: Defines HTTP routes:
        *   `/healthz`: Health check endpoint. Reports healthy (200 OK) only if the initial cache load is successful and the `gmd_syncer` is considered operational (e.g., successfully subscribed to Realtime events and/or heartbeat is satisfactory). Returns 503 Service Unavailable otherwise.
        *   `/metrics`: Prometheus metrics scraping endpoint.
        *   `/v1/indexes/{LogicalIndexName}/*`: Primary route for customer search requests. Chains API key auth, authorization, and routing resolver middleware, followed by the reverse proxy handler.

*   **`internal/middleware/`:**
    *   **`logger.go`:** Logs request details (method, path, status, duration, user agent, API key ID) using `zerolog`.
    *   **`metrics.go`:** Collects Prometheus metrics (request counts, latencies, response codes).
    *   **`recovery.go`:** Recovers from panics in HTTP handlers, logs the panic, returns a 500 error.
    *   **`apikey_auth.go`:**
        *   Extracts API key from `X-Meili-API-Key` header or `Authorization: Bearer <token>`.
        *   Looks up the (hashed or original) key in the `internal/cache`.
        *   Validates if the key exists, is not revoked, and is active.
        *   If valid, stores `CustomerID`, `permissions_json`, and `logical_index_access_pattern` (from cache entry) into the request `context`.
        *   Returns 401/403 if authentication fails.
    *   **`authorization.go`:**
        *   Retrieves `CustomerID`, `permissions_json`, `logical_index_access_pattern` from the request `context`.
        *   Extracts `LogicalIndexName` from the URL path parameter.
        *   **`logical_index_access_pattern` Matching:**
            *   **Purpose:** Restricts an API key's access to a defined set of `LogicalIndexName`(s).
            *   **Format:** String stored in `ApiKeys` table (GMD).
            *   **Supported Patterns & Logic (Phase 1 MVP):**
                1.  **Exact Match:** Pattern is the exact `LogicalIndexName`. (e.g., `my_specific_index`)
                2.  **Prefix Match with Wildcard:** Pattern ends with `*`. (e.g., `products_*` matches `products_main`, `products_staging_europe`). Wildcard only at the end.
                3.  **Global Wildcard:** Pattern is `*`. Grants access to all `LogicalIndexName`(s) for the `CustomerID`.
            *   **Implementation:**
                *   If pattern == `*`, allow.
                *   If `strings.HasSuffix(pattern, "*")`, then `strings.HasPrefix(LogicalIndexName, strings.TrimSuffix(pattern, "*"))` must be true.
                *   Else, `LogicalIndexName == pattern` must be true.
                *   If no match, return **403 Forbidden**.
        *   **`permissions_json` Enforcement:**
            *   **Purpose:** Defines granular actions allowed on matched `LogicalIndexName`(s).
            *   **Format:** JSONB object in `ApiKeys` table (GMD).
            *   **Structure (Meilisearch - Phase 1):**
                ```json
                {
                    "search": true,
                    "add_documents": true,
                    "get_documents": true,
                    "update_documents": true,
                    "delete_documents": true,
                    "get_settings": true,
                    "update_settings": true,
                    "reset_settings": true,
                    "get_stats": true,
                    "get_tasks": true,
                    "dump_access": false
                }
                ```
                (Missing keys imply `false`.)
            *   **Action Derivation (Mapping HTTP Request to Permission Key):**
                *   `POST` or `GET` to `/v1/indexes/{LogicalIndexName}/search` -> requires `"search": true`
                *   `POST` or `PUT` to `/v1/indexes/{LogicalIndexName}/documents` -> requires `"add_documents": true`
                *   `GET` to `/v1/indexes/{LogicalIndexName}/documents` or `.../documents/{doc_id}` -> requires `"get_documents": true`
                *   `POST` to `.../documents/delete-batch` or `DELETE` to `.../documents/{doc_id}` -> requires `"delete_documents": true`
                *   `GET` to `/v1/indexes/{LogicalIndexName}/settings/*` -> requires `"get_settings": true`
                *   `POST`, `PUT`, `PATCH` to `/v1/indexes/{LogicalIndexName}/settings/*` -> requires `"update_settings": true`
                *   `DELETE` to `/v1/indexes/{LogicalIndexName}/settings/*` -> requires `"reset_settings": true`
                *   `GET` to `/v1/indexes/{LogicalIndexName}/stats` -> requires `"get_stats": true`
                *   `GET` to `/v1/indexes/{LogicalIndexName}/tasks/*` -> requires `"get_tasks": true`
            *   This mapping uses `http.Request.Method` and path matching (e.g., regex on path suffix after `/v1/indexes/{LogicalIndexName}/`).
            *   If the derived action key is not present in `permissions_json` or is `false`, return 403 Forbidden.
        *   **Security Note:** FRR *must not* allow requests to system-level Meilisearch API endpoints not index-specific (e.g., `/health`, `/version`, global key creation) unless explicitly permitted (e.g. `dump_access`). The `/v1/indexes/{LogicalIndexName}/*` route is the primary conduit.
        *   Returns 403 if authorization fails.
    *   **`routing_resolver.go`:**
        *   Retrieves `CustomerID` (from context) and `LogicalIndexName` (from path).
        *   Looks up this combination in `internal/cache` to find `PhysicalInstance` details: `PhysicalIndexName`, `DeployedEngineID`.
        *   Uses `DeployedEngineID` to find `SearchNodeVM.ip_address`, `DeployedEngine.port`, `DeployedEngine.engine_type`.
        *   Stores resolved backend details (`ip_address`, `port`, `PhysicalIndexName`, `engine_type`) into request `context`.
        *   Returns 404 if mapping doesn't exist, or 500 for inconsistency.

*   **`internal/cache/cache.go`:**
    *   Implements in-memory cache using Go maps protected by `sync.RWMutex`.
    *   Stores:
        *   API key data: `map[string]ApiKeyDetails{ CustomerID, PermissionsJSON, LogicalIndexAccessPattern, Revoked }` (key is API key string).
        *   Routing data: `map[LogicalIndexKey]PhysicalInstanceDetails{ PhysicalIndexName, DeployedEngineID, SearchNodeIP, SearchNodePort, EngineType }` (where `LogicalIndexKey` might be a struct or string combining `CustomerID` and `LogicalIndexName`).
    *   Provides methods: `Get`, `Set`, `Delete`, `ReplaceAll` (for full sync from GMD).

*   **`internal/datasync/gmd_syncer.go`:**
    *   Responsible for keeping `internal/cache` synchronized with GMD.
    *   **A. Initial Data Load (on FRR Startup):**
        *   **Purpose:** Populate cache with a complete snapshot before serving requests.
        *   **Process:**
            1.  Connect to Supabase (PostgreSQL).
            2.  Execute SQL queries:
                *   **Query 1: `ApiKeys`:**
                    ```sql
                    SELECT key_hash, customer_id, permissions_json, logical_index_access_pattern, revoked
                    FROM ApiKeys WHERE revoked = FALSE;
                    ```
                    (Store keyed by `key_hash` in API key cache).
                *   **Query 2: Joined Routing Data (`LogicalIndexes`, `PhysicalInstances`, `DeployedEngines`, `SearchNodeVMs`):**
                    ```sql
                    SELECT
                        li.customer_id,
                        li.logical_name,
                        pi.physical_index_name_on_vm,
                        de.id AS deployed_engine_id,
                        de.engine_type,
                        de.port,
                        snvm.ip_address
                    FROM LogicalIndexes li
                    JOIN PhysicalInstances pi ON li.id = pi.logical_index_id
                    JOIN DeployedEngines de ON pi.deployed_engine_id = de.id
                    JOIN SearchNodeVMs snvm ON de.search_node_vm_id = snvm.id
                    WHERE li.status = 'active' AND pi.status = 'active' AND de.status = 'running' AND snvm.status = 'active';
                    ```
                    (Process and populate routing cache, keyed by e.g., `{ CustomerID, LogicalName }`).
            3.  **Cache Update:** Use `cache.ReplaceAll()` for atomic replacement.
            4.  **Error Handling:** Log errors. Retry loop with backoff. FRR main HTTP server should not serve requests (except unhealthy `/healthz`) until load succeeds and `gmd_syncer` is operational.
    *   **B. Realtime Updates (via Supabase Realtime):**
        *   **Purpose:** Dynamically update cache with GMD changes post-initial load.
        *   **Mechanism:** Use `nedpals/supabase-go` client to subscribe to INSERT, UPDATE, DELETE on `ApiKeys`, `LogicalIndexes`, `PhysicalInstances`, `DeployedEngines`, `SearchNodeVMs`.
        *   **Processing Events:**
            *   **`ApiKeys` Table:** Add/Update/Delete corresponding entries in the API key cache.
            *   **`LogicalIndexes`, `PhysicalInstances`, `DeployedEngines`, `SearchNodeVMs` Tables (Routing-related):**
                *   **Simplified Strategy for MVP:** On *any* relevant change event (INSERT, UPDATE, DELETE) from these four routing-related tables, **trigger a full re-query and `ReplaceAll` of the *entire routing cache*** (using a similar query to the initial load for routing data).
                    *   *Rationale:* Simplifies event processing logic significantly, maintains correctness through atomic replacement. Less prone to errors from complex delta logic.
                    *   *Trade-off:* Less efficient for the sync process if routing data is massive and updates are very frequent, but acceptable for MVP and prioritizes robustness.
        *   **Error Handling & Resilience (Realtime):** Log connection/subscription/processing errors. Supabase client expected to handle reconnections.
    *   **C. Heartbeat/Staleness Check (Secondary Verification):**
        *   **Purpose:** To detect if the FRR's cache has become significantly stale due to potential silent failures or missed messages in the Supabase Realtime stream.
        *   **Mechanism:**
            1.  A dedicated goroutine in `gmd_syncer` will run periodically (e.g., every 1-5 minutes, configurable).
            2.  **Query GMD:** Fetch a frequently updated timestamp from the GMD (e.g., `MAX(updated_at)` from a key table like `PhysicalInstances`, or a dedicated heartbeat table).
            3.  **Compare Timestamps:** Compare this GMD timestamp with an internal FRR timestamp representing when it last successfully processed a Realtime event from Supabase (or last successful heartbeat sync).
            4.  **Staleness Detection:** If `GMD_timestamp` is significantly newer than `FRR_last_realtime_event_timestamp` (by a configurable threshold, e.g., > 2-3 times the heartbeat interval), the cache is considered potentially stale.
        *   **Action on Staleness Detection:**
            1.  **Log Critical Error:** Generate a high-severity log indicating potential cache staleness and the observed time delta.
            2.  **Trigger Full Cache Reload:** Immediately initiate a full data reload from the GMD for *all* cached data (`ApiKeys` and routing data), similar to the initial startup load, using `cache.ReplaceAll()`.
            3.  **Increment Metrics:** Increment a Prometheus counter (e.g., `frr_cache_staleness_reloads_total`).
            4.  Ensure the Realtime subscription continues or is re-established to catch subsequent changes.
        *   **Rationale:** This acts as a "deadman's switch," providing a crucial secondary verification layer for cache consistency, enhancing reliability against rare Realtime stream failures.

*   **`internal/proxy/reverse_proxy.go`:**
    *   Uses `net/http/httputil.NewSingleHostReverseProxy`.
    *   **Custom `Director` function:**
        1.  Retrieve resolved backend details (`ip_address`, `port`, `PhysicalIndexName`, `engine_type`) from request `context`.
        2.  Dynamically set `req.URL.Scheme`, `req.URL.Host` (to `ip_address:port`), and `req.URL.Path` (rewriting `LogicalIndexName` to `PhysicalIndexName`, e.g., `/v1/indexes/my_products/search` -> `http://{ip_address}:{port}/indexes/customerID_my_products/search`). `engine_type` informs specific rewrite rules if they differ (Phase 3).
        3.  Set `req.Host` to target host.
        4.  Pass through most headers. FRR *does not* inject Meilisearch Master Keys.
    *   **Custom `ModifyResponse` function:** Omitted for MVP unless a specific, known need to strip/modify sensitive internal backend headers arises. Default pass-through is sufficient.
    *   **Custom `ErrorHandler`:** Handle proxy errors (e.g., backend unreachable), return appropriate 5xx responses.

## 7. Test-Driven Development (TDD) Approach

TDD will be central to building each component:

1.  **Define smallest unit of functionality.**
2.  **Write a Test:** Create a unit test for expected behavior (initially failing). Use table-driven tests. Mock dependencies (e.g., cache for middleware, GMD for `gmd_syncer`).
3.  **Write Code:** Minimal code to make the test pass.
4.  **Run Tests:** Verify all tests pass.
5.  **Refactor:** Improve code structure, readability, performance (tests still pass).
6.  **Repeat:** For new functionalities, edge cases, error conditions.

**Example - TDD for `middleware/apikey_auth_test.go` (conceptual):**
```go
// TestAPIKeyAuth_ValidKey: Input valid key, mock cache returns details. Expected: Next handler called, context populated.
// TestAPIKeyAuth_InvalidKey: Input key not in mock cache. Expected: 401, next handler not called.
// TestAPIKeyAuth_MissingKey: Input no key. Expected: 401, next handler not called.
// TestAPIKeyAuth_RevokedKey: Input valid key, mock cache says revoked. Expected: 403, next handler not called.
// TestAPIKeyAuth_BearerTokenFormat: Input valid "Authorization: Bearer <token>", mock cache valid. Expected: Next handler, context populated.
```
**Integration Testing Focus:**
Beyond unit tests, integration tests in the `tests/` directory will be crucial for verifying:
*   End-to-end request flow through the middleware chain and proxy.
*   `gmd_syncer` behavior during initial load and in response to simulated GMD changes.
*   Resilience scenarios, such as GMD unavailability during startup or Realtime connection disruptions and recovery via heartbeat/staleness checks.
*   Correctness of routing and authorization based on cache state.
