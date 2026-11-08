# OpenCode Go runner guard

This package isolates OpenCode Go reviews inside the existing runner. The application supports
Claude, Codex and Go; Go accounts require an explicitly supplied API key through the authenticated
Settings flow. Legacy OpenCode credentials are not reused. Public settings and runner selection
validate a confirmed selection against the current official Go catalogue. Five custom tools exist:
`ocr_read`, `ocr_grep`, `ocr_glob`, `ocr_bash`, and `ocr_write`. Native tools remain removed and denied.

The package targets the checksum-verified OpenCode CLI **2.2.6**. Its dependency-free server entry uses that version's promise plugin API. Each tool executor calls the fixed `/usr/local/bin/ocr-sandbox opencode-tool` binary with an empty environment, bounded stdin/stdout and a three-second timeout. The Rust bridge validates a strict request, calls the existing `guard_tool`, and opens files through the archive's descriptor-relative, no-symlink opener. There is no native-tool delegation or optional security hook.

- `ocr_read`: absolute path, optional one-based offset and limit of 1–200 lines; regular UTF-8 files at most 256 KiB.
- `ocr_grep`: absolute root and literal pattern at most 256 bytes; sorted `path:line:text` results, at most 80 matching lines. CRLF evidence is preserved.
- `ocr_glob`: absolute root and a relative pattern at most 256 bytes. Supports `*`, `?`, bracket classes, and `**` as an entire directory segment matching zero or more directories. Returns sorted regular-file paths. Parent/absolute patterns are denied.
- Search traversal skips symlinks and special entries, rejects explicitly symlinked roots, and allows at most 4096 entries and depth 32. Grep additionally scans at most 4 MiB. File/output budgets fail closed rather than returning an incomplete result.
- `ocr_bash`: accepts exactly the existing `read_commands::validate_input` schema and grammar. The shared parser returns a typed plan for bounded sed reads, literal grep searches, or directory listings. The bridge interprets that plan directly; **no shell or external command receives model text**. No `cwd`/`workdir`, network, background jobs, or sandbox override is accepted. The fixed working directory is `/output`; the helper deadline remains three seconds regardless of the validated optional timeout hint.
- `ocr_write`: only `/output/report.json` and `/output/finding_decisions.json`, at most 64 KiB UTF-8 content. Rejects existing symlinks, hardlinks and special targets. Uses a descriptor-pinned output directory, a private exclusive temporary file, and atomic rename. A concurrent target swap cannot redirect writes into the target inode. Temporary names are implementation-owned, not model-selected paths; completed/error paths clean them up. SIGKILL interruption cleanup remains a runner-stage responsibility.

Requests are capped at 512 KiB, stdout at 300 KiB, and stderr at 4096 bytes. All results are text only. Oversized, malformed, failed, cancelled, or timed-out helper execution is a tool denial.

The helper now returns a versioned envelope: exit 0 with `{"v":1,"status":"ok","content":"..."}`, exit 2 with `{"v":1,"status":"rejected","code":"request"}`, or exit 70 with `{"v":1,"status":"fault","code":"internal"}`. Ordinary model-request rejection is not a stage infrastructure failure. Spawn, timeout, signal, invalid status/envelope, overflow and required-evidence probe failures permanently fault the stage.

`health.js` connects to one root-supervised Unix socket using a fresh nonce supplied only in the isolated CLI environment. The runner checks Linux peer UID and nonce. Frames are limited to 1 KiB. Setup installs hooks but does not attest the legitimately empty bootstrap model inventory. The first resolved `model.request` validates the final model inventory and five-tool registry, probes fixed required evidence, repeats validation and awaits the root readiness ACK. `http.request` awaits the same readiness promise before sending. The pinned, awaited `experimental.idle` hook repeats those checks, seals the adapter and obtains root acknowledgement of completion for the sole transcript session. Sealing without readiness or work after sealing faults the stage. Cleanup before sealing faults; orderly cleanup afterward does not. Final text, a successful process exit, or startup readiness alone never authorizes a report.

`opencode.json` is a system configuration template for **`/etc/opencode/opencode.json`**, not a global `OPENCODE_CONFIG_DIR` file. It must be installed root-owned and mounted read-only with the package. The trusted HOME/config roots must also be read-only; state/cache/data have separate writable directories. The smoke test emulates those boundaries with private mounts, not host `/etc` writes. The whitelist includes `opencode.models.dev` because that exact built-in materializes the temporary V2 provider map; it does not enable dynamic provider packages or native tools.

## Dynamic catalogue and temporary V2 contract

The backend discovers models only through an unauthenticated, bounded request to
`https://opencode.ai/inference/go/v1/models`, without redirects or a production URL override.
There is no shipped Go model list or default model. Saved model/effort selections require catalogue
confirmation. Invalid or ambiguous multi-transport entries are excluded individually; valid entries
remain available. Availability and account eligibility can change independently.

Only values explicitly advertised in `reasoning.effort` are selectable efforts. Budget/toggle
capabilities do not create efforts. A model without advertised efforts uses an empty effort and no
`#` suffix; this does not mean that the model cannot reason. Reasoning budgets are independent of
visible output limits. Variants invented by the CLI normalizer are removed. CLI 2.2.6's internal
`default` variant sentinel is accepted only for this no-effort case, with no final variants and no
reasoning controls on the wire; it is not a model default or a selectable effort.

Review preparation freezes a typed selected-model snapshot or its error. An unusable optional Go
backup does not block Claude/Codex: the frozen error is propagated only when Go is selected, before
Go inference. Root independently validates the snapshot and writes a selected-model-only V2 map at
`/scratch/session/go-models.json`, root-owned with mode `0444`, plus its contract. The map contains
only `opencode-go`, an empty provider environment-key list and the selected model's public metadata;
it contains no credential or pricing. No production static `models.json` remains.

The CLI performs no catalogue discovery. `OPENCODE_MODELS_PATH` points to that temporary map and
`OPENCODE_DISABLE_MODELS_FETCH=1` disables remote fetching, but that flag alone does not prevent
embedded fallback. The guard validates the file, normalized model and final inventory, requiring
exactly the selected provider/model and permitted variants. Missing files, embedded fallback and
unexpected inventory fail closed. Post-transform `Model.Info.settings.apiKey` must separately match
the root-projected `OCR_OPENCODE_GO_KEY`; credential bytes are never catalogue data or diagnostics.

Before sending, the final `http.request` attestation checks the serialized protocol, official HTTPS
endpoint, model and reasoning controls against the frozen contract. Anthropic's pinned
`block_binding` is allowed only as `{prefix_mismatch_behavior:"drop_block"}`. Catalogue validation,
final inventory, readiness ACK and wire attestation are all required; none substitutes for another.

## Privacy and account limits

Go sends evidence through OpenCode and its upstream model providers. DeepSeek uses global
processing. All Contributor models, including Muse Spark and Iceman, require checking the current
contributor terms and obtaining the required data-sharing consent. Listing a model never grants
consent. Do not use confidential code without the required approval.

The app does **not** enforce subscription-only usage. Go Extra Usage can incur additional charges
under the account's current settings and terms. An API key does not prove plan entitlement, quota,
or remaining allowance. Quota and reset information remain unknown; no balance or reset is invented.

## Offline verification

The runner fake implements Chat Completions, Responses, Anthropic Messages and Google
GenerateContent. The matrix covers four representatives, not every discovered model or effort, and does
not establish account eligibility or commercial availability. Run guard, runner and targeted
authentication, redaction, cancellation and evidence-identity regressions separately.

Namespace proof does not replace matching-image verification: check installed bytes, immutable
assets, fingerprints, egress, container restrictions and the complete evidence-validation pipeline.

Run the offline proof on Linux after obtaining the verified CLI artifact separately:

```sh
cargo build -p opencodereview-server --bin ocr-sandbox --locked
python3 -B docker/opencode-smoke.py --cli /path/to/verified/2.2.6/opencode
python3 -B docker/opencode-runner-smoke.py --cli /path/to/verified/2.2.6/opencode
```

The test needs `unshare`, `ip`, Bubblewrap, and Python's standard library. It creates its own user/network namespaces and private temporary mount roots. It never invokes a container engine, downloads packages, calls a real provider, or reads the user's OpenCode configuration. There is no external interface in the test network namespace. Fault fixtures wrap the real executor or break its helper; they do not substitute a mock guard. Available cases cover all five executors, prohibited paths/commands/writes, traversal budgets, target-swap races, hostile config, plugin failures/unload, helper failures, and cancellation during a running helper. Cancellation allows at most 3.5 seconds for the CLI's three-second ownership-lease shutdown. These available cases are not a claim that the complete suite passed against the current dynamic guard. Container boundaries, real credential transport and deployment remain separate gates requiring oracle review.

The second script additionally needs configured subordinate UID mappings (`unshare --map-auto`). It runs the actual Rust runner, drops to UID/GID 10001, installs a real frozen archive over IPC, and invokes the pinned CLI against a non-forwarding fake API. It exercises health rejection, selected-key redaction, state cleanup, 429 cancellation, cancellation/disconnect recovery and idle-probe failure when run. Namespace mode does not prove container cgroups/seccomp. Container mode is prepared with `--image <reconciled-private-runner-image> --provider-image <already-local-python-image> --engine docker|podman`; it never builds or pulls and publishes no ports. It uses only uniquely owned disposable resources.

Authentication termination uses the awaited `http.response` hook, not NDJSON timing. An actual primary Go HTTP 401 for `ocr-review` repeats the same registry and required-evidence checks as idle, seals the adapter, and awaits a `provider_failed` acknowledgement. Root requires that authenticated receipt, a matching sole transcript session, a terminal structured `provider.auth`/401 event, and normal CLI exit 1. Earlier faults, later work, duplicate terminals, output/redaction errors, cancellation and survivors prevent failover. `http.request` rejects retries after sealing. No response body or credential enters the health channel.

Run `--check-auth-failover` for pinned-CLI authentication and adversarial termination checks when
execution is authorized. Namespace proof does not replace matching-image container smokes or
security review. These fake-provider tests do not authorize paid inference.
## Offline protocol fixtures

`python3 -B docker/test-opencode-protocol-fixtures.py` checks the four synthetic
HTTP/SSE codecs and static harness seams with Python stdlib only. It does not run
OpenCode, the sandbox, containers, or inference.

Integration owner can run the existing smoke with an already-built sandbox and
the pinned CLI:

```sh
python3 -B docker/opencode-runner-smoke.py --binary /absolute/ocr-sandbox --cli /absolute/opencode --protocol responses --check-model
python3 -B docker/opencode-runner-smoke.py --binary /absolute/ocr-sandbox --cli /absolute/opencode --protocol-matrix
```

The matrix runs sequentially: `kimi-k3#max` (Chat), `gpt-6-luna#medium`
(Responses), `claude-haiku-5-5#medium` (Anthropic), and
`gemini-4-argon#medium` (Gemini). Each checks a guarded tool result, terminal
report, two usage steps totaling uncached input 28/output including reasoning
14/cache read 6, initial and post-tool HTTP 401/429, request counts, and forged
authentication text. This covers four protocols, not all models or variants.
Existing adversarial smoke flags remain available; the matrix does not repeat
the entire adversarial suite for every representative.

Synthetic snapshots retain the official HTTPS origins, paths and SDKs. In namespace mode,
the isolated fake terminates TLS CONNECT using an ephemeral test CA; it does not rewrite snapshot
origins. Unknown paths/models are rejected. The fake binds only to loopback inside an isolated
network, never forwards, and uses only synthetic sentinel credentials. The container fixture mount is writable
solely for request counts and contract-failure markers. No real provider requests
or captured recordings are used; no CLI NDJSON is generated by these codecs.
Container mode still requires explicit already-local runner and Python images.
Passing these commands does not establish commercial entitlement or model privacy consent.

### Current proof scope

With the pinned real CLI and rebuilt sandbox, the current dynamic guard passed the four-protocol
namespace matrix described above. Additional real CLI proofs passed for a new synthetic Chat model
without effort, an Anthropic model advertising budget/toggle but no effort list, and a new Chat
model with the advertised synthetic effort `experimental-fast`. Focused
`node docker/test-opencode-catalogue.mjs` regressions passed for deferred acknowledged readiness,
final inventory, credential mismatch, no-effort, wire tampering, missing/fallback data and cleanup.
The four Python codec selftests also passed; those selftests alone are not runner integration.

Historical static-catalogue proofs and hashes do not attest this dynamic guard. The complete
adversarial suite has not been rerun for this revision. Matching production image bytes,
fingerprints, egress/container acceptance and final security review remain pending. The expanded
13-scenario Rust failover test, including provider-local frozen Go errors, also remains unexecuted.
Namespace proofs mount current JS assets and do not establish matching production images or
authorize production activation or paid inference.
