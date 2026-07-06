# Subprocessors & data processing

This page is the **published, maintained subprocessor list** that accompanies our
Data Processing Agreement (DPA). It names the third parties that may process
customer data on our behalf, what each one does, and what data it sees — so a
security or privacy reviewer can assess the chain without a sales call.

!!! note "Scope"
    A *subprocessor* is a third party we engage that may process **customer
    personal data** in the course of providing the service. Open-source
    libraries, vendored binaries that run on your own machine (git, the Pi
    runtime), and tools that never touch customer data are **not** subprocessors.

## Data Processing Agreement

We offer a **standard DPA** to customers who need one; request it through
[support](support.md). The DPA incorporates this subprocessor list by reference
and commits us to the change-notice process below.

## Change notice

We maintain this list and give **advance notice of changes** (GDPR Art. 28(2)
style): before a new subprocessor begins processing customer data, we update this
page and notify customers who have subscribed to subprocessor-change notices, so
you have a window to review or object before it takes effect.

## Current subprocessors

The list is **seeded from the infrastructure the product actually uses** and grows
as vendors are adopted. Status badges reflect whether a dependency is live today
or tied to a capability that is still rolling out.

| Subprocessor | Purpose | Data it may process | Status |
|---|---|---|---|
| **Microsoft Azure** | Hosted data plane, the confidential-VM boundary, and the Key Vault KMS that wraps data-at-rest keys. | Hosted workspace content (behind handles), wrapped encryption keys, operational metadata. | <span class="status planned">Hosted/attested tier</span> |
| **LLM inference providers** — OpenAI, Anthropic, Azure OpenAI | **Managed** inference when you use a platform-provided model rather than your own linked account. | The prompts and in-scope context sent for a run, in plaintext (see [Where your data goes](../concepts/protection.md#where-your-data-goes)). | <span class="status available">When managed inference is used</span> |
| **Payment processor** — Stripe | Billing and the consultant↔client settlement rail. | Billing contact + transaction metadata. Card data is entered directly into the processor; **we never see or store it**. | <span class="status planned">With the settlement rail</span> |
| Operational vendors (email delivery, error/uptime monitoring, etc.) | Running and supporting the service. | Operational metadata; added here **before** they process customer data. | Added as adopted |

## Bring-your-own model accounts reduce the chain

If you link your **own** LLM account (OAuth or API key) instead of using managed
inference, the inference relationship is **yours, not ours** — that provider is
*your* subprocessor, not one we interpose. This is a privacy and disclosure win:
your prompts go to a provider you already have a contract with.

!!! info "Status"
    Linking a model account is supported for API keys today; the **OAuth
    link flow** that stores a sealed credential as account state is in progress
    (it rides the account-model work). Until then, managed inference uses the
    providers listed above.

## See also

- [Security & trust](../security.md) — the overall model and the reviewer-grade documents.
- [Where your data goes](../concepts/protection.md#where-your-data-goes) — the plaintext-exposure map.
- [Support & response targets](support.md) — how to reach us, including the security/abuse contact.
