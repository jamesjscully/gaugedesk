// SAML SP verify sidecar (M3 ID-2). Reads one JSON request on stdin and writes one
// JSON verdict on stdout — the wire contract the Rust SamlSidecarIdentityProvider
// drives (crates/app/src/identity_saml.rs). The XML-dsig / C14N / XSW-safe reference
// checking is owned by @node-saml/node-saml; this script only adapts the I/O and
// normalizes attributes. Fail-closed: any error → {ok:false}.
//
//   request : { "saml_response": <base64 POST-binding SAMLResponse>,
//               "idp_cert": <IdP signing cert PEM>,
//               "audience": <SP entity id> }
//   verdict : { "ok": true, "subject": <NameID>, "attributes": { name: [values] } }
//           | { "ok": false, "error": <reason> }

import { SAML } from "@node-saml/node-saml";

function emit(obj) {
    process.stdout.write(JSON.stringify(obj));
}

async function readStdin() {
    const chunks = [];
    for await (const chunk of process.stdin) chunks.push(chunk);
    return Buffer.concat(chunks).toString("utf8");
}

// Extract the assertion's unique ID and its expiry, so the Rust side can enforce
// single-use (replay rejection). node-saml validates the timestamps but does NOT
// dedupe assertion IDs across logins, so this is the data its complementary
// one-time-use cache needs. The ID is the per-assertion `@ID`; the expiry prefers the
// SubjectConfirmationData NotOnOrAfter (the tightest delivery bound), else Conditions.
// On any extraction failure the id is "" — which the Rust side treats as fail-closed.
function assertionMeta(profile) {
    let id = "";
    let notOnOrAfterMs = null;
    const first = (v) => (Array.isArray(v) ? v[0] : v); // node-saml mixes arrays/scalars
    try {
        // getAssertion() returns the parsed document; the assertion is under `.Assertion`.
        const doc = profile.getAssertion && profile.getAssertion();
        const a = first(doc && doc.Assertion);
        if (a && a.$ && a.$.ID) id = String(a.$.ID);
        const subject = a && first(a.Subject);
        const sc = subject && first(subject.SubjectConfirmation);
        const scd = sc && first(sc.SubjectConfirmationData);
        const cond = a && first(a.Conditions);
        const naStr =
            (scd && scd.$ && scd.$.NotOnOrAfter) || (cond && cond.$ && cond.$.NotOnOrAfter);
        if (naStr) {
            const ms = Date.parse(naStr);
            if (!Number.isNaN(ms)) notOnOrAfterMs = ms;
        }
    } catch {
        // leave id empty → Rust rejects fail-closed (cannot guarantee single-use)
    }
    return { id, notOnOrAfterMs };
}

// node-saml puts each attribute on profile.attributes as a scalar (one value) or an
// array (many). The Rust contract is always name → string[]; normalize here.
function normalizeAttributes(profile) {
    const out = {};
    const attrs = profile && profile.attributes;
    if (attrs && typeof attrs === "object") {
        for (const [name, value] of Object.entries(attrs)) {
            out[name] = Array.isArray(value) ? value.map(String) : [String(value)];
        }
    }
    return out;
}

async function main() {
    let req;
    try {
        req = JSON.parse(await readStdin());
    } catch {
        return emit({ ok: false, error: "invalid request json" });
    }
    const { saml_response, idp_cert, audience } = req || {};
    if (!saml_response || !idp_cert) {
        return emit({ ok: false, error: "missing saml_response or idp_cert" });
    }

    let saml;
    try {
        saml = new SAML({
            idpCert: idp_cert,
            issuer: audience || "gaugewright-sp",
            // Check the assertion's AudienceRestriction contains our SP entity id.
            audience: audience || false,
            callbackUrl: "http://localhost/saml/acs",
            // Stateless verify: we hold no prior AuthnRequest id to match.
            validateInResponseTo: "never",
            // Require the assertion to be signed; the outer Response need not be.
            wantAssertionsSigned: true,
            wantAuthnResponseSigned: false,
        });
    } catch (e) {
        return emit({ ok: false, error: "config: " + String((e && e.message) || e) });
    }

    try {
        const { profile } = await saml.validatePostResponseAsync({ SAMLResponse: saml_response });
        if (!profile || !profile.nameID) {
            return emit({ ok: false, error: "no subject in assertion" });
        }
        const meta = assertionMeta(profile);
        emit({
            ok: true,
            subject: String(profile.nameID),
            attributes: normalizeAttributes(profile),
            // Single-use replay defense data for the Rust side (INV-20/INV-21).
            assertion_id: meta.id,
            not_on_or_after: meta.notOnOrAfterMs,
        });
    } catch (e) {
        // Signature failure, audience mismatch, expired conditions, malformed XML — all
        // land here and become a rejection (fail-closed). No XML is trusted on error.
        emit({ ok: false, error: String((e && e.message) || e) });
    }
}

main();
