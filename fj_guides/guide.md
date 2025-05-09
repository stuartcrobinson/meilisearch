Okay, this is a great set of information! Let's consolidate it into a single, well-structured document describing Flapjack Search.

## Flapjack Search: Managed Search Service

**Brief Summary:**

Flapjack Search is a managed web service designed to simplify the deployment, scaling, and operation of search engines like Meilisearch (and potentially Typesense in the future) for application developers. It provides users with geo-distributed search instances, abstracting away the complexities of server infrastructure management and search engine operations. A core technical differentiator is its custom-forked Meilisearch engine supporting Single Index Snapshots (SIS), enabling efficient, fine-grained index mobility for horizontal scaling and resource optimization, all transparently managed via the Flapjack Request Router (FRR) API gateway, accessed via geo-optimised DNS endpoints. Users can also define Index Groups, collections of Logical Indexes guaranteed to be co-located on the same physical search engine instance, primarily for multi-index search capabilities.

---

**Technical Summary (Approx. 500 words):**

Flapjack Search aims to deliver a "search-as-a-service" platform, initially focusing on Meilisearch. The architecture is designed to provide developers with the power of Meilisearch without the operational overhead, offering scalability, multi-tenancy, and ease of management.

At the heart of the customer interaction is the **Flapjack Request Router (FRR)**. This globally distributed API gateway, running on dedicated **FRRNodeVMs** and made accessible to users via geo-based DNS routing (e.g., AWS Route 53 Geolocation or Latency-based routing policies), serves as the sole entry point for all customer search requests. The FRR's primary responsibilities include HTTPS termination, robust stateless API key authentication (e.g., JWTs or opaque tokens validated against the Global Master Database), and fine-grained authorization. It maps an API key to a specific customer and ensures that requests are only permitted for `LogicalIndexName`(s) the customer owns, enforcing permissions (stored as `permissions_json` in the GMD) like read-only or read-write access. A crucial function of the FRR is dynamic routing: each FRR instance maintains its own high-speed in-memory cache (e.g., a Go map with `sync.RWMutex`) of routing information. This cache is kept updated from the Global Master Database via **FRR Data Sync** (primarily Supabase Realtime subscriptions). This routing information allows the FRR to map a customer's `LogicalIndexName` (e.g., `my_products`) to the correct backend search engine. It does this by finding the `PhysicalInstance` which points to a `DeployedEngine` (giving `port`, `engine_type`, and `engine_version`) which in turn points to a `SearchNodeVM` (giving `ip_address`). The FRR also gets the `PhysicalIndexName` (e.g., `customerID_my_products`) from the `PhysicalInstance`. The FRR transparently rewrites incoming request paths (e.g., `/v1/indexes/my_products/search`) to the internal physical path before proxying the request to the correct backend search engine instance. This ensures that users interact with Flapjack as if they are interacting directly with a standard Meilisearch API, with the FRR handling the multi-tenancy and routing invisibly.

The **Core Service Layer** consists of multiple search engine instances (initially Meilisearch), each running on a dedicated **SearchNodeVM**. A key innovation is the use of a custom-forked Meilisearch that implements **Single Index Snapshot (SIS)** functionality. This custom API allows Flapjack to create a standard Meilisearch `.snapshot` file of an individual index, transfer it, and import it onto a target `DeployedEngine` (which resides on a `SearchNodeVM`). This SIS process is fundamental for efficiently moving specific customer indexes between physical servers. This enables dynamic scaling (moving a hot index to a more powerful `SearchNodeVM`), resource optimization (co-locating smaller indexes), and seamless maintenance. Tenancy within a single search engine instance is achieved by prefixing index names with the customer's ID to form the `PhysicalIndexName`.

Orchestrating the entire system is the **Orchestration & Management Layer**. Central to this is the **Global Master Database (GMD)**, which will be **Supabase (PostgreSQL)**. The GMD is the single source of truth for all metadata. This includes customer details, API keys and their permissions (`permissions_json`), `LogicalIndexName` definitions, and `Index Group` definitions. It also tracks `SearchNodeVMs` (the virtual machines), `DeployedEngines` (specific search engine instances like Meilisearch or Typesense running on a `SearchNodeVM`, including their `engine_type`, `version`, and `port`), and `PhysicalInstances` (which map a `LogicalIndexName` to a `DeployedEngine` and a `PhysicalIndexName` on that engine). The GMD also stores status and capacity metrics for `SearchNodeVMs` and `FRRNodeVMs`. The FRRs keep their routing caches updated from the GMD via FRR Data Sync. An **Orchestrator Service** (initially scripted, later a robust automated service) manages the lifecycle of `SearchNodeVMs` and `DeployedEngines`, and executes the SIS-based index migration workflows. This includes provisioning new `SearchNodeVMs`, deploying search engines (creating `DeployedEngines` records), triggering snapshots, managing secure file transfers of snapshots, initiating imports on target `DeployedEngines`, and then updating the GMD, which in turn signals FRRs to refresh their caches via FRR Data Sync.

The **Infrastructure Layer** will utilize cloud VMs (EC2, Hetzner, etc.) for distinct `FRRNodeVMs` and `SearchNodeVMs`, with plans for bare metal for `SearchNodeVMs`. It includes public static IPs for regional FRR clusters, internal networking, DNS management, and storage for search engine data and temporary snapshot transfers (e.g., S3).

Future enhancements (Phase 2) include a **Customer Control Plane** (web dashboard and API) for self-service management of `LogicalIndex`(es) and `Index Group`(s), full automation of the Orchestrator, comprehensive monitoring and alerting, and advanced security hardening. Phase 3 plans for the expansion to support other search engines like Typesense, applying similar architectural principles and leveraging the `engine_type` field in the `DeployedEngines` table. The FRR is designed to be high-performance, with Go being a preferred language, but will rely on external services like Cloudflare or AWS Shield for DDoS mitigation rather than attempting to build this capability internally.

---

**Detailed Technical Guide:**

This section outlines the various components, features, technologies, and services required to build Flapjack Search, broken down by layer and phase.

**Phase 1: Core Meilisearch Service MVP**

**I. Customer-Facing Layer**

   **A. Flapjack Request Router (FRR) - The "Brain"**
      *   **Purpose:** Secure public API gateway for all customer search requests. Handles authentication, authorization, routing, and tenant isolation, while making the interaction feel like directly using Meilisearch.
      *   **Key Features:**
          1.  **HTTPS Termination:** Securely terminates incoming customer HTTPS connections.
          2.  **Authentication & Authorization:**
              *   API Key validation: Stateless preferred (e.g., JWTs or opaque tokens validated against the Global Master Database). Accepts Meilisearch-standard `X-Meili-API-Key` header or `Authorization: Bearer <token>`.
              *   Mapping API Key to `CustomerID` & allowed `LogicalIndexName`(s)/Patterns.
              *   Enforcing index-level permissions via `permissions_json` stored in GMD (e.g., `{"search": true, "add_documents": true}`).
              *   Preventing access to unauthorized indexes or system-level Meilisearch endpoints not intended for customer use.
          3.  **Request Ingestion & Basic Sanitization:**
              *   Basic input validation (e.g., against obviously malicious payloads, malformed requests). DDoS mitigation will primarily rely on external services (Cloudflare, AWS Shield).
          4.  **Dynamic Routing Logic:**
              *   **Lookup Process:**
                  1.  From API key, derive `CustomerID`.
                  2.  Using `CustomerID` and `LogicalIndexName` (from URL path), find the `LogicalIndex` record.
                  3.  From the `LogicalIndex` record, find the corresponding `PhysicalInstances` record.
                  4.  From `PhysicalInstances.deployed_engine_id`, look up the `DeployedEngines` record. This provides the `engine_type`, `port`, `engine_version`, and `search_node_vm_id`.
                  5.  From `DeployedEngines.search_node_vm_id`, look up the `SearchNodeVMs` record to get the `ip_address` (specifically, the internal IP used for backend communication).
                  6.  The FRR now has: `ip_address`, `port`, `engine_type`, and the `PhysicalIndexName` (from `PhysicalInstances.physical_index_name_on_vm`).
              *   Routing information (derived from GMD tables like `LogicalIndexes`, `PhysicalInstances`, `DeployedEngines`, `SearchNodeVMs`) is sourced from the FRR's local in-memory cache, kept synchronized with the Global Master Database via FRR Data Sync.
          5.  **Request Transformation (Search Engine Specific):**
              *   **URL Path Rewriting:** Rewrite customer-facing URL paths to match internal API paths on the target search engine instance.
                  *   Example (Meilisearch): Customer sends request to `/v1/indexes/{LogicalIndexName}/search`.
                  *   FRR, using the resolved `ip_address`, `port`, and `PhysicalIndexName`, rewrites and proxies to: `http://{ip_address}:{port}/indexes/{PhysicalIndexName}/search`.
                  *   The `engine_type` from `DeployedEngines` can be used to adjust transformations if other engines (like Typesense) have different API path structures.
              *   **Meilisearch Master Key Handling:** The FRR *will not* inject Meilisearch Master Keys per request. Backend Meilisearch instances will be configured with a shared, internal-only master key managed by Flapjack. The FRR acts as a trusted client forwarding requests.
          6.  **Response Handling:** Forward the Meilisearch response (data, status codes, headers) back to the customer, maintaining the feel of direct Meilisearch interaction. Error responses for auth/authz issues will also mimic Meilisearch error formats where appropriate.
        7.  **Geo-Routing (Initial Strategy):**
            *   DNS-based geo-routing (e.g., utilizing AWS Route 53 Geolocation or Latency-based routing policies) directs customers to the regional FRR cluster anticipated to provide the lowest latency, typically by routing them to the FRR cluster geographically closest to them.
            *   The FRR within that accessed regional cluster then routes the request to the customer's designated primary (or secondary, if applicable) backend search engine, which could reside in a different geographical region based on the customer's data residency or performance choices.
      *   **Technology Stack Ideas:**
          *   **Primary Language:** Go (Golang) for high performance, excellent concurrency, and strong networking libraries.
          *   **HTTP Framework (Go):** Standard `net/http`, or lightweight frameworks like Chi, Gin, or Echo.
          *   **Reverse Proxy Core:** `net/http/httputil.NewSingleHostReverseProxy` (customized Director and ModifyResponse functions).
          *   **In-memory Cache:** Per FRR instance: Built-in Go map with `sync.RWMutex` for routing information and API key details. This is the primary cache for hot-path requests. Redis is not used for this.
          *   **Local Cache Persistence (Optional Optimization):** FRR *may* persist its in-memory cache to local disk on the `FRRNodeVM` for faster restarts; GMD via FRR Data Sync remains the source of truth.

**II. Core Service Layer (Search Engine Hosting)**

   **A. Search Engine Instances (e.g., Meilisearch):**
      *   **Deployment:** One search engine process per **SearchNodeVM**.
      *   **Custom Fork (Meilisearch):** Flapjack's version of Meilisearch with custom **Single Index Snapshot (SIS)** API endpoints enabled.
      *   **Configuration (per instance on `SearchNodeVM`):**
          *   `data.ms` directory (for Meilisearch) or equivalent for other engines.
          *   Dedicated snapshot directory for SIS output.
          *   Master Key (Meilisearch): A single, shared internal master key for all Flapjack-managed Meilisearch instances. This key is *not* customer-facing.
          *   Listen address (from the `SearchNodeVMs.ip_address` of the VM hosting the engine) and port (from the `DeployedEngines.port` specific to this engine instance).
      *   **Tenancy within Instance:** Indexes will be named with a customer prefix to form the `PhysicalIndexName` (e.g., `customerA_products`, `customerB_articles`) to ensure uniqueness within a single search engine instance. The FRR translates the `LogicalIndexName` to this `PhysicalIndexName`.

   **B. Single Index Snapshot (SIS) Process (Meilisearch specific):**
      *   **Mechanism:** Leverages Flapjack's custom Meilisearch SIS API endpoints.
      *   **Output:** Aims to produce standard Meilisearch `.snapshot` files.
      *   **Workflow (Orchestrated):**
          1.  **Trigger Snapshot:** Orchestrator calls the SIS API on the source Meilisearch instance (on its `SearchNodeVM`) for a specific `PhysicalIndexName` (e.g., `customerA_products`).
          2.  **Secure Transfer:** The `.snapshot` file is securely transferred from the source `SearchNodeVM` to a target `SearchNodeVM`.
              *   Methods: `scp` or `rsync` over SSH (simpler start). For very large files or more robust transfers, use an intermediary object storage (e.g., S3 presigned URL upload from source, download to target).
          3.  **Import Snapshot:** Orchestrator calls the SIS import API on the target Meilisearch instance, providing the path to the transferred snapshot file. This creates/replaces the index on the target instance.
          4.  **Update GMD:** Orchestrator updates the `PhysicalInstances` record for the `LogicalIndexName` in the Global Master Database, specifically by updating its `deployed_engine_id` to point to the new target `DeployedEngine` where the index now resides.
          5.  **Signal FRRs (via FRR Data Sync):** Orchestrator signals FRRs to update their routing cache (e.g., by ensuring GMD change triggers Supabase Realtime event).

**III. Orchestration & Management Layer (Initially Manual/Scripted, then Automated)**

   **A. Global Master Database / Configuration Store (GMD):**
      *   **Purpose:** Single source of truth for all service metadata.
      *   **Chosen Technology:** Supabase (PostgreSQL).
      *   **Data Model (Illustrative Tables):**

          *   **`Customers`**
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `name` (TEXT)
              *   `billing_info_id` (TEXT) - Could be FK to a billing system
              *   `status` (TEXT) - e.g., 'active', 'suspended'

          *   **`ApiKeys`**
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `key_hash` (TEXT UNIQUE NOT NULL) - Securely hashed API key.
              *   `customer_id` (UUID FK REFERENCES `Customers`(id))
              *   `permissions_json` (JSONB) - Granular permissions. Example: `{"search": true, "add_documents": true, "update_settings": false}`
              *   `logical_index_access_pattern` (TEXT) - e.g., `products_*`, `specific_index_name` (for restricting key scope)
              *   `revoked` (BOOLEAN DEFAULT false)
              *   `description` (TEXT)

          *   **`IndexGroups`**
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `customer_id` (UUID FK REFERENCES `Customers`(id))
              *   `name` (TEXT) - Customer-defined name for the group.
              *   UNIQUE (`customer_id`, `name`)

          *   **`LogicalIndexes`**
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `customer_id` (UUID FK REFERENCES `Customers`(id))
              *   `index_group_id` (UUID FK REFERENCES `IndexGroups`(id), NULLABLE) - For co-locating indexes.
              *   `logical_name` (TEXT NOT NULL) - Customer-facing index name (e.g., `my_products_index`).
              *   `primary_region` (TEXT)
              *   `secondary_region_optional` (TEXT)
              *   `status` (TEXT) - e.g., 'active', 'creating', 'deleting'
              *   UNIQUE (`customer_id`, `logical_name`)

          *   **`SearchNodeVMs`** (Describes the Virtual Machine itself)
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `ip_address` (INET) - Internal IP address.
              *   `hostname` (TEXT, NULLABLE) - Optional: A human-readable name. Operationally, hostnames should be treated as unique for clarity in logging/monitoring, but this is not strictly DB-enforced to allow flexibility if provisioning already ensures it or uses cloud-generated names.
              *   `region` (TEXT)
              *   `capacity_metrics_json` (JSONB) - e.g., CPU, RAM, disk specs.
              *   `status` (TEXT) - e.g., 'active', 'maintenance', 'provisioning', 'decommissioned'.

          *   **`DeployedEngines`** (Links a specific search engine deployment to a `SearchNodeVM`)
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `search_node_vm_id` (UUID FK REFERENCES `SearchNodeVMs`(id) ON DELETE CASCADE)
              *   `engine_type` (TEXT NOT NULL) - e.g., 'meilisearch', 'typesense'.
              *   `engine_version` (TEXT NOT NULL)
              *   `port` (INTEGER NOT NULL) - Port this engine instance is listening on.
              *   `status` (TEXT) - e.g., 'running', 'stopped', 'error', 'deploying', 'unhealthy'.
              *   `config_details_json` (JSONB) - Optional: For engine-specific configurations managed by Flapjack that aren't covered by dedicated columns (e.g., specific runtime flags, plugin settings, or minor tuning parameters).
              *   UNIQUE (`search_node_vm_id`, `port`)
              *   UNIQUE (`search_node_vm_id`, `engine_type`) - Assuming one instance of a given engine type per VM.

          *   **`PhysicalInstances`** (Maps a `LogicalIndex` to a `DeployedEngine` where its data resides)
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `logical_index_id` (UUID FK REFERENCES `LogicalIndexes`(id) ON DELETE CASCADE)
              *   `deployed_engine_id` (UUID FK REFERENCES `DeployedEngines`(id) ON DELETE RESTRICT) - Points to the specific engine instance.
              *   `physical_index_name_on_vm` (TEXT NOT NULL) - Internal, namespaced name on the engine (e.g., `cust123_my_products_index`).
              *   `status` (TEXT) - e.g., 'active', 'migrating', 'standby', 'provisioning', 'failed'.
              *   `last_known_size_bytes` (BIGINT)
              *   UNIQUE (`deployed_engine_id`, `physical_index_name_on_vm`)

          *   **`FRRNodeVMs`** (Describes the VMs running the Flapjack Request Router)
              *   `id` (UUID PK DEFAULT gen_random_uuid())
              *   `public_ip_address` (INET)
              *   `internal_ip_address` (INET)
              *   `hostname` (TEXT, NULLABLE) `- Optional: A human-readable name for operators, logging, and monitoring. Operational uniqueness is vital for clarity but not strictly DB-enforced, allowing flexibility if provisioning already ensures it or uses cloud-generated unique names.`
              *   `region` (TEXT)
              *   `version` (TEXT) - FRR software version.
              *   `capacity_metrics_json` (JSONB)
              *   `status` (TEXT)

   **B. FRR Data Sync (Formerly FRR Update Mechanism):**
      *   **Primary Method:** FRRs subscribe to Supabase Realtime updates on relevant GMD tables (e.g., `PhysicalInstances`, `ApiKeys`, `LogicalIndexes`, `DeployedEngines`, `SearchNodeVMs`) to keep their in-memory caches current.

   **C. Orchestrator Service (Conceptual - Start with Scripts):**
      *   **Purpose:** Automate provisioning, scaling (index moves via SIS), health checks, and healing for `SearchNodeVMs` and `FRRNodeVMs`.
      *   **Initial Implementation:** A set of well-documented scripts (e.g., Bash, Python, Go) run by operators.
      *   **Key Actions (to be automated later):**
          1.  Provisioning a new `SearchNodeVM` or `FRRNodeVM` (via cloud provider API), creating corresponding records in GMD.
          2.  Deploying/configuring search engine services (custom fork for Meilisearch) on a `SearchNodeVM` (creating/updating `DeployedEngines` records), or FRR application on `FRRNodeVM`.
          3.  Initiating SIS process (trigger snapshot on a source `DeployedEngine`, orchestrate snapshot file transfer between source and target `SearchNodeVMs`, trigger import on a target `DeployedEngine`).
          4.  Updating the Global Master Database (e.g., new `deployed_engine_id` for a `PhysicalInstance`, VM/engine statuses, versions in `DeployedEngines`).
          5.  Ensuring GMD changes trigger FRR Data Sync (e.g., via Supabase Realtime).
          6.  Monitoring basic `SearchNodeVM` health, `DeployedEngine` health (e.g., checking Meilisearch `/health` endpoint), and `FRRNodeVM` health.
          7.  Decommissioning `DeployedEngines` and `SearchNodeVMs`, ensuring data is migrated or properly handled.

**IV. Infrastructure Layer**

   **A. Compute:**
      *   Distinct Virtual Machines for `FRRNodeVMs` and `SearchNodeVMs` (AWS EC2, Hetzner Cloud, DigitalOcean Droplets, Linode). No co-location of FRR and search engine processes on the same VM in production.
      *   Future: Bare metal servers for performance-critical `SearchNodeVMs`.
   **B. Networking:**
      *   Public static IPs for FRR clusters (running on `FRRNodeVMs`) per region.
      *   Internal networking (e.g., VPC, private networks) for `FRRNodeVMs` to reach `SearchNodeVMs`, and for `SearchNodeVMs` to communicate for snapshot transfers if not using S3.
      *   DNS management:
               *   **Public DNS:** For customer-facing FRR endpoints (e.g., `search.flapjack.com`), configured with geo-routing policies (e.g., AWS Route 53 Geolocation or Latency-based routing policies) to direct users to the optimal regional FRR cluster.
               *   **Internal DNS (Optional but Recommended):** For service discovery between internal components (e.g., Orchestrator finding GMD, FRRs resolving internal management endpoints if any).
   **C. Storage:**
      *   `SearchNodeVM` local disk (high-performance SSDs) for search engine data (e.g., Meilisearch `data.ms`).
      *   Temporary storage for snapshot transfers (e.g., an S3 bucket, or local disk space on `SearchNodeVMs` if using direct `scp`/`rsync`).
      *   Persistent storage for the Global Master Database (typically handled by the managed DB provider).

**Phase 2: Enhancements & Production Hardening**

**I. Customer-Facing Layer Enhancements**

   **A. Customer Control Plane (Web Dashboard & API):**
      *   **Purpose:** Allow customers to self-manage their Flapjack Search service.
      *   **Features:**
          *   User registration & authentication (e.g., using Auth0, Supabase Auth, custom solution).
          *   Provision new `LogicalIndex`(es) and manage `IndexGroup`(s).
          *   Manage API keys (create, list, revoke, set permissions via `permissions_json`).
          *   View basic usage statistics (query count, data size – aggregated from search engine stats via Orchestrator polling GMD or search engines directly).
          *   Select geo-regions for their `LogicalIndex`(es) (if offered).
          *   Billing information & plan management.
      *   **Technology Ideas:**
          *   Web Framework: Next.js, SvelteKit, Django, Ruby on Rails.
          *   API: REST or GraphQL, interacting with the Orchestrator Service API or directly with GMD (with appropriate security).

**II. Orchestration & Management Automation**

   **A. Automated Orchestrator Service:**
      *   Develop the scripts from Phase 1 into a robust, long-running, stateful service (e.g., in Go, Python).
      *   Implement job queues for managing asynchronous tasks like SIS.
      *   **Triggers for Scaling/Index Moves:**
          *   **Metrics-based:** CPU/memory usage on `SearchNodeVMs`, query latency/volume per index, index data size (thresholds defined in GMD or Orchestrator config).
          *   **Scheduled or manual triggers:** Via Control Plane API or internal admin commands.
      *   **Resource Management:**
          *   Track `SearchNodeVM` and `FRRNodeVM` capacities and utilization.
          *   Identify underutilized `SearchNodeVMs` for consolidating new/small indexes.
          *   Identify over-utilized `SearchNodeVMs`/indexes needing migration or new `SearchNodeVM` provisioning.
          *   Bin-packing algorithms for efficient index placement.

   **B. Monitoring & Alerting System:**
      *   **Metrics Collection:**
          *   **`SearchNodeVM` & `FRRNodeVM` level:** CPU, RAM, disk I/O & space, network (e.g., Prometheus Node Exporter, cloud provider metrics).
          *   **Search Engine-level (e.g., Meilisearch):** Query stats (count, latency), indexing stats, error rates, task queue length (from engine's `/stats` and `/health` endpoints). Orchestrator periodically polls these and stores aggregated/relevant metrics in GMD or a dedicated time-series DB.
          *   **FRR-level:** Request latency, error rates (4xx, 5xx), throughput, cache hit/miss rates (for its in-memory cache).
          *   **Global DB (Supabase):** Query performance, connection count, replication lag.
      *   **Tools:** Prometheus & Grafana, Datadog, or other managed observability platforms.
      *   **Alerting:** On critical thresholds (`SearchNodeVM` or `FRRNodeVM` down, search engine unresponsive, high error rates from FRR or search engine, full disk, GMD issues, SIS failures). PagerDuty, Slack integrations.

**III. Infrastructure & Reliability Enhancements**

   **A. Load Balancing for FRRs:** If an FRR cluster (multiple `FRRNodeVMs`) in a region has multiple instances, use a standard cloud load balancer (e.g., AWS ALB/NLB, Nginx).
   **B. Backup & Disaster Recovery (DR):**
      *   Regular, automated backups of the Global Master Database (Supabase handles this, but verify retention and DR procedures).
      *   Consider periodic full snapshots of all search engine data (beyond SIS for moves) stored in a separate, secure location (e.g., cross-region S3) for DR purposes. This is a heavier operation than SIS.
   **C. Security Hardening:**
      *   Intrusion Detection Systems (IDS/IPS).
      *   Web Application Firewall (WAF) for FRRs and Customer Control Plane (e.g., Cloudflare WAF, AWS WAF).
      *   Regular security audits, penetration testing.
      *   Secrets management: HashiCorp Vault, cloud provider KMS (e.g., for storing GMD credentials, internal API keys).
      *   Principle of Least Privilege for all service accounts and internal communications.
      *   Network segmentation.

**Phase 3: Expansion (Example: Typesense Support)**

*   **FRR Adaptation:**
    *   Modify FRR to understand Typesense API endpoints, authentication mechanisms (Typesense API keys), and request/response structures, potentially based on `engine_type` derived from GMD.
    *   Develop routing logic specific to Typesense if it differs significantly from Meilisearch's index-centric model (e.g., collection aliases).
*   **SIS Equivalent for Typesense:**
    *   **R&D Required:** Investigate Typesense's native capabilities for snapshotting and restoring individual collections or groups of collections. This would be the equivalent single-unit snapshot and restore mechanism for Typesense collections (e.g., a "Typesense Collection Snapshot" - TCS).
    *   If a direct TCS equivalent isn't available, explore alternatives:
        *   Export/Import APIs if efficient enough.
        *   Filesystem-level snapshotting (e.g., LVM snapshots) if Typesense can cleanly recover, though this is less granular.
        *   Potentially contribute to Typesense or develop a custom solution if critical.
*   **Global DB & Orchestrator Updates:**
    *   Utilize the `engine_type` field within the `DeployedEngines` table (which is itself linked to `SearchNodeVMs`).
    *   The GMD schema (`DeployedEngines`, `PhysicalInstances`) is already designed to support different `engine_type`s.
    *   Update Orchestrator logic to provision, manage (using `engine_type` from `DeployedEngines`), and (if TCS-equivalent exists) migrate Typesense collections/instances.
*   **Control Plane Updates:**
    *   Allow customers to choose and provision Typesense services (selecting 'typesense' as engine type) alongside Meilisearch.
    *   Display Typesense-specific metrics and management options.

This detailed outline provides a comprehensive roadmap for building Flapjack Search, starting with a core MVP and iteratively adding features and robustness.
