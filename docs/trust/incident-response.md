# Incident-response runbook

What to do when a security incident is suspected — a compromised credential,
unauthorized access, tampered history, or a data-handling mistake. This is a lean,
honest runbook for the current scale; it grows with the team.

A **security incident** is any suspected unauthorized access to, disclosure of, or
modification of customer data or the systems that hold it.

## 1. Detect

Incidents surface from the signals in
[security operations](security-operations.md):

- an audit-integrity check (`GET /admin/audit/verify`) returning `ok: false`;
- a rising SIEM-export `failure_count` / non-zero `dropped_count`;
- SIEM detections (auth-failure spikes, off-hours or out-of-scope access,
  export-gate denials);
- a report to the **security & abuse contact** (see [support](support.md)).

## 2. Triage & declare

- Confirm it is a real event (not a misconfiguration or test).
- Record the **time, scope (which tenant/data), and what is known** in an incident
  log. Assign one owner to coordinate.
- Classify severity by data sensitivity and blast radius.

## 3. Contain & isolate

Act to stop ongoing exposure first:

- **Revoke the actor.** Deactivate the member (`DELETE`/deactivate via the admin
  console or SCIM) — `Org::role_of` only returns active members, so deactivation
  revokes standing immediately. Revoke project grants (`DELETE /admin/grants`).
- **Rotate credentials.** Rotate the SCIM token (`POST /admin/scim/token` — rotation
  invalidates the prior token), IdP client secrets, and any KMS/service-principal
  credential that may be exposed.
- **Cut the network path** at the edge (proxy/WAF) if the control plane itself is
  implicated.
- **Preserve evidence** before remediation: the audit log is append-only and the
  source of truth — do not prune it. Export it (`GET /admin/audit?format=csv|json`)
  and snapshot the store for forensics.

## 4. Eradicate & recover

- Remove the root cause (patch, revoke, reconfigure).
- Restore from a known-good backup if data integrity is in question; verify the
  audit chain (`/admin/audit/verify`) on the restored state.
- Confirm the actor's access is gone and monitoring is clean before reopening.

## 5. Notify

- Notify affected customers per the **DPA** and applicable breach-notification law
  (timelines and contacts live in the DPA / customer agreement, not here).
- Keep the [subprocessor list](subprocessors.md) and customers informed if a
  subprocessor was involved.

## 6. Post-incident review

- Write a blameless post-mortem: timeline, root cause, what detection/containment
  worked, and concrete follow-ups (new detections, new controls, doc updates).
- File the follow-ups as tracked work.

---

This runbook is operational guidance; the load-bearing legal/process artifacts
(DPA, breach-notification terms, SOC 2 procedures) are maintained separately and
take precedence where they specify timelines or obligations.
