# Security operations & deployment hardening

How a GaugeWright deployment is configured and monitored to meet its security
posture. This is the operator-facing companion to the controls built into the
product — it names the **knobs you must set for a hosted/enterprise deployment**
and the **signals you should watch**.

Desktop/solo installs need none of this: they run on the operator's own machine,
on loopback, with sensible defaults. The settings below apply when you host the
control plane for others.

## Deployment hardening checklist (hosted / enterprise)

Set these before exposing a control plane beyond loopback. Several are **fail-loud
gates** — the process refuses to start rather than run insecurely.

| Setting | Purpose | Enforcement |
|---|---|---|
| `GAUGEWRIGHT_ALLOW_NETWORK_HTTP=1` | Acknowledge a non-loopback bind (the API has no cross-authority auth of its own — see below). | Refuses to start without it (FED-2). |
| `GAUGEWRIGHT_TLS_TERMINATED=1` | Confirm a TLS-terminating reverse proxy fronts the plain-HTTP API. | Non-loopback bind refused without it; every response also carries HSTS (ENTSEC-7). |
| `GAUGEWRIGHT_HARDENED=1` | Declare a hosted/hardened deployment. | Requires crash-safe durability below; warns if at-rest encryption is unconfigured (SECAUD-9). |
| `GAUGEWRIGHT_SQLITE_SYNCHRONOUS=FULL` | Crash-durable persistence (fsync per commit) so recent admits survive an OS crash. | Required under `GAUGEWRIGHT_HARDENED=1` (SECAUD-9). |
| `GAUGEWRIGHT_AUDIT_READS=1` | Also record **reads** of project-scoped data (transcripts/files/resources) to the audit trail — "who read this client's data". | Opt-in (reads are high-volume); off by default (SECAUD-4). |
| `GAUGEWRIGHT_ENCRYPT_CONTENT=1` | Encrypt transcript content at rest under **per-engagement** keys (`SECAUD-9`); deleting a chat **crypto-erases** its content (`SECAUD-6`, GDPR erasure). | Off by default. Dev uses a local KEK; a hosted deployment swaps in the KMS `KeyWrap` adapter. |

### Authentication boundary

The control-plane HTTP API authenticates **org members** (per-request, fail-closed)
once an identity provider is configured (enterprise mode); it has **no
cross-authority auth** — peers federate over the broker with cert-pinned TLS, not
this API. So a hosted control plane **must** sit behind:

- a **TLS-terminating reverse proxy** (the API speaks plain HTTP; tokens and
  transcripts must not cross the wire in cleartext), and
- an **edge rate-limiter / WAF**. In-process throttling exists as defense-in-depth
  (e.g. the SCIM bearer locks out after repeated failures, SECAUD-8), but the
  **primary** brute-force and abuse control is the edge. Configure per-IP request
  limits on the auth and data routes.

### Encryption at rest

Transcript **content** can be encrypted at rest under per-engagement keys by setting
`GAUGEWRIGHT_ENCRYPT_CONTENT=1` (`SECAUD-9`); deleting a chat then **crypto-erases**
its content — the key is destroyed, the retained ciphertext is unrecoverable
(`SECAUD-6`, GDPR right-to-erasure), the append-only log intact. Metadata/lifecycle
records remain plaintext (content-only by design).

The per-engagement DEKs are wrapped by a KEK. **Selecting the KMS is creds-only — no
code change:**

- **Dev / single-machine:** unset `GAUGEWRIGHT_CONTENT_KEK_ID` ⇒ a persisted local KEK.
- **Hosted (KMS):** set `GAUGEWRIGHT_CONTENT_KEK_ID` to the Key Vault KEK id
  (`https://<vault>.vault.azure.net/keys/<name>/<version>`) **and** the Azure *Crypto
  User* service-principal creds `AZURE_TENANT_ID` / `AZURE_CLIENT_ID` /
  `AZURE_CLIENT_SECRET`. The control plane then wraps every DEK via Key Vault
  `wrapKey`/`unwrapKey` (`SEC-4`). **Fail-loud:** if the KEK id is set but the creds are
  incomplete, or the vault is unreachable at startup, the process **refuses to start**
  rather than silently downgrading to a local KEK.

For full at-rest coverage of the rest of the data plane (and git-blob workspace files),
also run on an **encrypted volume / KMS-backed disk**. `GAUGEWRIGHT_HARDENED=1` warns if
no at-rest KEK is configured.

## Monitoring & alerting

The product emits the signals; your monitoring stack consumes and alerts on them.

- **Audit integrity.** `GET /admin/audit/verify` returns the hash-chain integrity of
  the governance audit log (`{ok, entries, head, broken_at, anchored}`, SECAUD-2). The
  head is **signed** by the governance key on every append, so `verify` catches not
  just mid-chain edits but also tail truncation, last-entry edits, and a forged
  checkpoint. **Alert if `ok` is false** (history was altered) **or if `anchored` is
  false** when you run signed (the signed anchor is missing). For belt-and-suspenders
  against a full dual-wipe of the log *and* its checkpoints, also export the `head`
  to an external witness.
- **SIEM export health.** The audit SIEM exporter retries a bounded backlog and
  exposes `failure_count` (export attempts that failed — the SIEM-unhealthy rate),
  `dropped_count` (entries lost to backlog overflow — an actual gap), and
  `pending_count` (export lag) (SECAUD-3). **Alert on a rising `failure_count` or any
  `dropped_count`.**
- **Audit stream.** Stream the audit log to your SIEM (Splunk HEC / Datadog / a
  webhook) and build detections there: a spike of role changes, repeated auth
  failures, an export-gate denial, or off-hours access.
- **Liveness.** `GET /health` is the readiness/liveness probe.

## Terraform state & secrets

Infrastructure (the `infra/` tree in the private `gaugewright-cloud` repo) is
provisioned with Terraform. **Never commit
`*.tfstate`** — it contains plaintext provider secrets and is gitignored. For any
shared/production infrastructure:

- Use a **remote state backend** (Azure Blob, S3, Terraform Cloud) with encryption
  at rest and access control, not local state files.
- **Rotate any credential** that has ever appeared in a local state file or other
  unencrypted artifact, even if it was never committed.
- Keep the [subprocessor list](subprocessors.md) current as infrastructure changes.

See also the [incident-response runbook](incident-response.md).
