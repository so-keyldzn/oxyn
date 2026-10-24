## Elasticsearch and OpenSearch Search DSL

Write a single REST request in the connection's `{{language}}` console format:
an upper-case HTTP method, one absolute API path, and an optional JSON body.
`POST` alone does not mean a write: route semantics decide it. Use the mapping
from context to choose fields and preserve JSON values as JSON; do not invent
SQL, shell commands, or an endpoint absent from the connection's product and
version.

For reads, bound result size and make sort order explicit when a user needs
repeatable navigation. For a potentially mutating route, state the scope and
the server-side consequences before proposing it. `_bulk`, by-query operations,
and reindexing can have partial effects: a client-side interruption never proves
that no item changed.
