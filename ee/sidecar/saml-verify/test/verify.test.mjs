// End-to-end test of the verify sidecar: mint a signed SAML Response with a
// self-signed cert, run verify.mjs over it, and assert it accepts a valid assertion
// and rejects tampered / wrong-cert / unsigned input (fail-closed, the security-
// critical direction).

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import selfsigned from "selfsigned";
import { makeSignedResponse } from "./make-fixture.mjs";

const VERIFY = fileURLToPath(new URL("../verify.mjs", import.meta.url));
const AUDIENCE = "gaugewright-sp";

function pem(attrs) {
    const r = selfsigned.generate(attrs || [{ name: "commonName", value: "idp.example.com" }], {
        keySize: 2048,
        algorithm: "sha256",
        days: 365,
    });
    return { cert: r.cert, key: r.private };
}

/** Run verify.mjs with a request object; resolve its parsed JSON verdict. */
function runVerify(request) {
    return new Promise((resolve, reject) => {
        const child = spawn("node", [VERIFY], { stdio: ["pipe", "pipe", "inherit"] });
        let out = "";
        child.stdout.on("data", (d) => (out += d));
        child.on("error", reject);
        child.on("close", () => {
            try {
                resolve(JSON.parse(out));
            } catch (e) {
                reject(new Error(`non-JSON verdict: ${out} (${e})`));
            }
        });
        child.stdin.end(JSON.stringify(request));
    });
}

test("accepts a valid signed assertion and maps subject + attributes", async () => {
    const { cert, key } = pem();
    const saml_response = makeSignedResponse({
        subject: "alice@acme.com",
        audience: AUDIENCE,
        attributes: { roles: ["admin", "member"], region: ["eu"] },
        certPem: cert,
        keyPem: key,
    });
    const verdict = await runVerify({ saml_response, idp_cert: cert, audience: AUDIENCE });
    assert.equal(verdict.ok, true, `expected ok, got: ${JSON.stringify(verdict)}`);
    assert.equal(verdict.subject, "alice@acme.com");
    assert.deepEqual(verdict.attributes.roles, ["admin", "member"]);
    assert.deepEqual(verdict.attributes.region, ["eu"]);
    // The replay-defense fields the Rust side needs for single-use enforcement.
    assert.equal(verdict.assertion_id, "_assertion-fixture-1", "assertion id is emitted");
    assert.equal(typeof verdict.not_on_or_after, "number", "expiry is emitted as epoch ms");
});

test("rejects a tampered assertion (signature no longer matches)", async () => {
    const { cert, key } = pem();
    const good = makeSignedResponse({
        subject: "alice@acme.com",
        audience: AUDIENCE,
        attributes: { roles: ["admin"] },
        certPem: cert,
        keyPem: key,
    });
    // Flip the subject inside the signed XML.
    const xml = Buffer.from(good, "base64").toString("utf8").replace("alice@acme.com", "evil@acme.com");
    const tampered = Buffer.from(xml, "utf8").toString("base64");
    const verdict = await runVerify({ saml_response: tampered, idp_cert: cert, audience: AUDIENCE });
    assert.equal(verdict.ok, false);
});

test("rejects a response signed by a different (untrusted) cert", async () => {
    const signer = pem();
    const other = pem([{ name: "commonName", value: "attacker" }]);
    const saml_response = makeSignedResponse({
        subject: "alice@acme.com",
        audience: AUDIENCE,
        attributes: {},
        certPem: signer.cert,
        keyPem: signer.key,
    });
    // Trust a DIFFERENT cert than the one that signed it.
    const verdict = await runVerify({ saml_response, idp_cert: other.cert, audience: AUDIENCE });
    assert.equal(verdict.ok, false);
});

test("rejects an unsigned response", async () => {
    const { cert } = pem();
    const unsigned = Buffer.from(
        `<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"><saml:Assertion><saml:Subject><saml:NameID>x</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>`,
        "utf8",
    ).toString("base64");
    const verdict = await runVerify({ saml_response: unsigned, idp_cert: cert, audience: AUDIENCE });
    assert.equal(verdict.ok, false);
});

test("rejects a malformed request", async () => {
    const verdict = await runVerify({ idp_cert: "x" });
    assert.equal(verdict.ok, false);
});
