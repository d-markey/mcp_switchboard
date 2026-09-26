<table width="100%">
<tr>
<td width="72">

<img src="assets/logo.svg" alt="mcp-switchboard logo" width="64" height="64">

</td>
<td>

# mcp-switchboard

***Concentrate your backends. Cut your tokens.***

</td>
</tr>
</table>

A byte-level reverse proxy that fronts one or more backend MCP servers
(streamable HTTP) and, optionally, trims what crosses the wire to save on
context tokens.

Built with **Tokio**, **Axum**, and **Reqwest** for maximum concurrency and efficiency.

- **Concentration.** Front several backend MCP servers with one process/port,
  each mounted at its own `/<name>/mcp` path. Every request — headers, body,
  session id, SSE framing — is relayed to its backend as-is. The client's MCP
  session *is* the backend's MCP session; the proxy just sits underneath as a
  relay, not a second, independent MCP server. Each mounted backend still
  behaves like a fully independent MCP server as far as the protocol goes —
  its own handshake, its own session id — so an agent connecting to several of
  them treats them as separate server entries, same as pointing at the
  backends directly.

- **Tool search** (opt-in, off by default). Instead of sending a backend's
  full tool list — every tool's name, description, and input schema — up
  front, expose just two proxy-provided tools, `describe_tools` and
  `call_tool`, so a client only pays for the schema of a tool it actually
  uses. See ["Tool search"](#tool-search-reducing-up-front-tool-schema-cost).

- **Response Compaction** (opt-in, off by default). A `tools/call` response's
  text content is inspected and rewritten to a more token-efficient format. 
  [**Markdown tables**](https://www.markdownguide.org/extended-syntax/#tables)
  selectively target tabular lists, while [**TOON**](https://toonformat.dev/) 
  (Token-Oriented Object Notation) systematically transforms any JSON payload 
  into a punctuation-minimized representation. See 
  ["Response Compaction"](#response-compaction-markdown--toon).

- **CORS** (off by default — irrelevant to a non-browser MCP client). Turn it
  on to let a browser-based MCP client call the proxy directly, even fronting
  a backend with no CORS support of its own. See
  ["CORS"](#cors-for-a-browser-based-client).

Everything else — other methods, errors, non-text content, the notification
stream, session termination, and any pre-2025-06-18 ("legacy") exchange —
passes through completely unchanged.

## Usage

```bash
cargo run --release -- --config PATH [--host HOST] [--port PORT]
                       [--ssl-keyfile PATH --ssl-certfile PATH]
```

By default the proxy speaks plain HTTP and listens on `127.0.0.1:8000`. 

`PATH` is a YAML file listing the named backends to proxy, each exposed at
its own `/<name>/mcp` path:

```yaml
# config.yaml
servers:
  jira:
    url: https://jira-mcp.example.com/mcp
    description: Jira issue tracker
  expert_tools:
    url: https://my-expert-mcp.example.com/mcp
    description: Expert tools
    rewrite: md_tables     # opt in to JSON/JSONL-to-markdown rewriting
    # rewrite: toon        # OR opt in to TOON (Token-Oriented Object Notation)
    # rewrite: csv         # OR opt in to CSV (with tab separator)
    use_tool_search: true  # opt in to tool search for this backend
    tool_prefix: expert    # optional; defaults to the server's own name ("expert_tools") above
    forward_origin: true   # forward a browser client's real Origin header to this backend
    headers:               # optional; default headers for each request to this backend
      Authorization: Bearer ${EXPERT_API_KEY}   # environment variable substitution
    tools:                 # optional; restrict which of this backend's real tools are exposed
      whitelist:           # glob patterns; omit/empty means "everything not blacklisted"
        - get_*
      blacklist:           # glob patterns; always wins over the whitelist
        - get_secrets

cors:                      # optional; omit entirely to disable CORS
  allow_origins:           
    - https://my-web-agent.example.com
```

**Injecting default headers.** Set `headers` on a backend to have the proxy
fill in fixed headers of its own whenever the client doesn't send a header
of that name itself. This is how you hold a backend's credential (an API key, 
a bearer token) in the proxy's config instead of it needing to live in every 
agent's own MCP config. A `${VAR}` in a header value is substituted from 
the proxy's own environment at load time.

**Restricting exposed tools.** Besides simply hiding tools you don't want 
exposed, tool filtering lets one backend URL be mounted more than once 
under different `servers` names, each restricted to its own tool subset:

```yaml
servers:
  expert_read:
    url: https://my-expert-mcp.example.com/mcp
    headers:
      Authorization: Bearer ${EXPERT_API_KEY} # pre-authorized
    tools:
      whitelist:
        - get_*
  expert_admin:
    url: https://my-expert-mcp.example.com/mcp
    # restricted tools fall through to here and require the caller's own
    # Authorization header, since none is set for this backend
    tools:
      blacklist:
        - get_*
```

## HTTPS and TLS Termination

The expected way to serve the proxy over HTTPS is to put a TLS-terminating 
gateway in front — nginx, Caddy, Traefik, or a cloud load balancer — forwarding 
plain HTTP to the proxy behind it. That's the common case and needs no 
proxy-side configuration.

For environments with no such gateway, pass `--ssl-keyfile` and 
`--ssl-certfile` to have the proxy terminate TLS itself using `rustls`. 
If you need to serve both HTTP and HTTPS at once, run two instances — 
one with the SSL flags, one without — pointed at the same config file.

## Tool search (reducing up-front tool-schema cost)

Applies only to a backend with `use_tool_search: true` set.

Normally, a backend's real `tools/list` result — full name, description,
and input schema for every tool — is sent to the client verbatim. Set
`use_tool_search: true` on a backend to replace that with two
proxy-provided tools instead:

* **`describe_tools`** — look up the description/input schema for one or
  more real backend tools by name, on demand.
* **`call_tool`** — actually invoke a real backend tool by name.

This trades a lookup round trip the first time a given tool is used for a
much smaller per-conversation token footprint.

### Tool search and `notifications/tools/list_changed`

A backend that supports it may send `notifications/tools/list_changed`
when its own tool set changes — a bare JSON-RPC notification with no
payload, just a signal that a client's cached `tools/list` is stale and
worth re-fetching. It arrives over the GET-based notification stream,
which the proxy never inspects, so it reaches the client unchanged
regardless of `use_tool_search`.

With `use_tool_search` on, a client that reacts to it the normal way — by
re-issuing `tools/list` — gets back a synthetic result built fresh from the
backend's *current* tool list (nothing here is cached), so
`describe_tools`'s description is regenerated with whatever tools exist at
that moment, including any that are new. There's no extra staleness for
the proxy to introduce, since it never cached the old list to begin with; a
client that ignores the notification and never refreshes simply won't
learn about the new tool, exactly as it wouldn't against the backend
directly.

## Response Compaction (Markdown & TOON)

Applies only to a backend with `rewrite: md_tables` or `rewrite: toon` set.

Only a *response* to a `tools/call` *request* is ever a rewrite
candidate. The proxy inspects the response's text content and, if it matches 
the selected mode's criteria, rewrites it to a more token-efficient format 
before forwarding it to the client.

Whenever `content` is actually rewritten, `structuredContent` is dropped from
the response to ensure the client model uses the compacted version for token 
savings. Only "modern" (2025-06-18+) exchanges are considered.

### Markdown Tables (`rewrite: md_tables`)
**Selective.** Only triggers when a `tools/call` response contains a JSON 
array of objects (or an object containing one) that meets a minimum "fill 
ratio" (0.5). It converts these lists into dense Markdown tables, 
declaring keys once as headers to eliminate syntax repetition across 
hundreds of rows. If the response isn't tabular, it's passed through 
unchanged.

See [Markdown tables](https://www.markdownguide.org/extended-syntax/#tables).

### TOON (`rewrite: toon`)
**Systematic.** Every valid JSON or JSONL response from a `tools/call` is 
reformatted into **Token-Oriented Object Notation**. TOON is a custom, 
whitespace-and-punctuation-minimized format designed for maximum token 
efficiency while preserving the semantic structure for the LLM. Unlike 
Markdown tables, TOON handles arbitrary nesting and non-tabular data 
consistently.

See [TOON](https://toonformat.dev/).

## CORS (for a browser-based client)

If your MCP client is a web page, add a top-level `cors` block naming the origins it's
served from. This answers preflight `OPTIONS` requests and adds the necessary
`Access-Control-*` headers (including `Access-Control-Expose-Headers` for `Mcp-Session-Id`).

The browser's `Origin` header is stripped by default to prevent backends from
rejecting the proxy request. Set `forward_origin: true` if the backend needs the real origin.

## Scalability

The proxy keeps no state of its own across requests: no session table, no
per-client bookkeeping, no cache. Session continuity is entirely the 
backend's affair — the `Mcp-Session-Id` handed out by the backend is 
relayed unchanged.

This means several identically-configured instances of the proxy can sit 
behind a plain round-robin load balancer with no shared state or sticky 
sessions needed between them.

Furthermore, CPU-bound tasks such as response compaction (Markdown tables 
and TOON generation) on payloads larger than 4KB are automatically offloaded 
to Tokio's multi-threaded blocking pool via zero-copy pointer transfers. This 
ensures that heavy payloads do not block the main asynchronous I/O worker 
threads, keeping the proxy highly responsive and capable of sustaining 
high throughput under heavy concurrent loads.

## Security Posture

These are deliberate, opinionated design decisions, not oversights — noted 
here so a deployer can make an informed call:

*   **No proxy-level authentication.** The proxy does not check who's calling 
    it; auth is the backend MCP server's job, same as if a client connected 
    to it directly. Deploy it somewhere behind your own network controls.
*   **No request timeout to backends.** The proxy won't cut off a slow 
    backend on its own; timing out a long-running call is the client/harness's 
    call to make, not the proxy's.
*   **No request/response size limits.** Large payloads are relayed as-is. 
    If a backend and its caller are fine exchanging something large, the 
    proxy won't be the reason it's dropped.
*   **No throttling or circuit breaking.** Throttling is the backend's job; 
    the proxy just relays the status code (e.g., `429`). Circuit breaking 
    is the calling harness's job.
*   **Credentials are the deployer's responsibility.** Never put a real 
    secret directly in a config file; substitute them from environment 
    variables using `${VAR}`.

## Development / Troubleshooting

Add `RUST_LOG=debug` (or `mcp_switchboard=debug`) to see each proxied request and response.

```bash
# Run unit tests
cargo test
```

### Project Structure

*   **[main.rs](src/main.rs)**: CLI entry point, argument parsing, and Axum server lifecycle management.
*   **[proxy.rs](src/proxy.rs)**: The core proxy engine. Handles request/response relaying, header forwarding, and routing to specific transformation handlers.
*   **[config.rs](src/config.rs)**: YAML configuration loading, validation, and environment variable substitution.
*   **[tool_search.rs](src/tool_search.rs)**: Implements the synthetic `describe_tools` and `call_tool` logic for the Tool Search feature.
*   **[tool_filter.rs](src/tool_filter.rs)**: Logic for tool whitelisting and blacklisting.
*   **[transform.rs](src/transform.rs)**: The dispatcher for response compaction, deciding whether to invoke Markdown or TOON rewriting.
*   **[transform_md_tables.rs](src/transform_md_tables.rs)**: Logic for converting tabular JSON data into dense Markdown tables.
*   **[transform_toon.rs](src/transform_toon.rs)**: Systematic recursive transformer for Token-Oriented Object Notation.
*   **[sse.rs](src/sse.rs)**: Server-Sent Events (SSE) relaying and stream-based transformation support.
*   **[jsonrpc.rs](src/jsonrpc.rs)**: Protocol helpers for matching JSON-RPC IDs and identifying tool call shapes.
*   **[backend_logging.rs](src/backend_logging.rs)**: Per-backend request/response logging with header redaction.

### Backend-specific logging
You can override the log level per backend in the config:

```yaml
servers:
  jira:
    url: https://jira.example.com/mcp
    log_level: debug
    log_headers:
      - X-Request-Id
```
Every header's *name* is logged, but its value is redacted (`<redacted>`)
by default. Use `log_headers` to name headers whose values are safe to log.
