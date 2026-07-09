# FAQ

Short, honest answers to the questions newcomers ask most. Every capability
claim below carries a status badge; the single source of truth for status is the
[Roadmap &amp; status](reference/status.md) table. For the safety model behind these
answers see [How GaugeWright protects your work](concepts/protection.md), and for
the words used here see the [glossary](concepts/glossary.md).

!!! warning "The one truth to read first"
    GaugeWright orchestrates **locally**, but inference is **remote**. The
    workbench runs on your machine; the agent's reasoning is performed by the
    **third-party LLM provider you configure**, so your prompts and the in-scope
    [context](concepts/glossary.md#context) are sent to that provider over
    the network. There is no local-only inference today.
    <span class="status available">Available</span>
    See [Where your data goes](concepts/protection.md#where-your-data-goes).

## Data, providers &amp; trust

??? question "Does my data stay on my machine?"
    Your project, [resources](concepts/glossary.md#resource), and the append-only
    history are stored **locally**.
    <span class="status available">Available</span>

    Sharing with another party — [federation](concepts/glossary.md#federation) —
    is opt-in, but it is **not operationally available end-to-end today**: it is
    implemented and exercised only in a loopback + NAT-isolated CI harness, not as
    a shippable cross-machine path.
    <span class="status built">Built</span>

    **But the agent's reasoning runs at the third-party LLM provider you
    configure** — so your prompts and the in-scope context are sent to that
    provider over the network. There is no local-only inference today.
    <span class="status available">Available</span>
    See [Where your data goes](concepts/protection.md#where-your-data-goes).

??? question "Which LLM providers can I use?"
    You configure your own provider (e.g. OpenAI, Anthropic, Azure OpenAI) and
    sign in to / supply credentials for it.
    <span class="status available">Available</span>

    With **your own** credentials, the LLM relationship is yours — the provider is
    *your* subprocessor, not GaugeWright's. Its retention and training terms are
    the provider's. If your data may not leave for a third-party model, use a
    provider you have contracted, or wait for **confidential inference** (removing
    the provider from the trust boundary), which is on the roadmap.
    <span class="status planned">Planned</span>

??? question "Who can see my plaintext?"
    Stated plainly:

    - **You can**, on your own machine. <span class="status available">Available</span>
    - **The LLM provider you configure can** — prompts and in-scope context are
      sent to it for inference. <span class="status available">Available</span>
    - **A relay cannot.** A *relay* is the transport role in
      [federation](concepts/glossary.md#federation) — it queues, retries, and
      forwards encrypted bridge messages between machines, but is never a payload
      authority and gains no payload access from carrying a handle. When you
      collaborate across machines, the relay routes only encrypted bytes and never
      reads payload (this is a machine-checked invariant, `INV-14`, not a policy
      promise). Cross-machine federation itself is <span class="status built">Built</span>
      (CI harness only), so this protection holds by construction but is not yet a
      shippable end-to-end path.
    - **GaugeWright (the company) cannot**: there is no hosted service holding your
      data today. (Cross-machine federation is not operationally available; see
      below.) <span class="status available">Available</span>

    See the full data-flow treatment in
    [How GaugeWright protects your work](concepts/protection.md).

## Pricing

??? question "What's free, and what's paid?"
    The current pricing direction separates **free/open-source capability** from
    **paid managed operation** and **paid enterprise governance**:

    | Use | Price |
    |---|---|
    | **Local, single-party** workbench (build · run · review) | **Free / open-source target** <span class="status available">Available</span> |
    | **Core protections** (handles, admission, review/release, export, audit structure) | **Free / open-source target** |
    | **Self-operated federation** (your own machines or your own relay) | **Free / open-source target** <span class="status built">Built</span> *(productized operation still limited)* |
    | **Account ledger / blind directory** (pubkeys, device registry, routing pointers, sealed account blob) | **Free managed exception** <span class="status planned">Planned</span> |
    | **Enterprise governance** (SSO, SCIM, RBAC, audit, admin controls, security policy, governed thin-client workspace) | **Paid enterprise / source-available target** <span class="status built">Built</span> <span class="status planned">live Planned</span> |
    | **Managed cloud services** (tokens, VMs, hosted workspaces, hosted relay, public embed host, KMS/SKR, attestation issuer) | **Paid managed service** <span class="status planned">Planned</span> |
    | **Attested sealed runs** (confidential VM, metered compute floor) | **Commercially metered** <span class="status built">verifier Built</span> <span class="status planned">live Planned</span> |

    !!! note "Self-federation vs. relayed multi-party"
        **Federation the mechanism** is free/open: the protocol, lifecycle, and
        self-operated path should remain inspectable and usable without a paid
        GaugeWright service. **GaugeWright-operated federation** is different: when
        our hosted relay, broker, or workspace carries a crossing between separate
        parties, that is a paid managed service. Relays still do not read payloads;
        metering bills operation, not content.

    The paid model has three parts:

    1. **Enterprise governance entitlement:** SSO, SCIM, RBAC, audit, admin
       controls, security policy, and governed thin-client workspace capabilities
       are paid enterprise features, with source available under commercial terms.
    2. **Managed cloud usage:** tokens, VMs, hosted workspaces, hosted relay,
       KMS/SKR, hosted embed sessions, and attestation infrastructure are paid
       because they consume cloud resources or create platform obligations.
    3. **Settlement / metering:** experts set their own engagement price, and
       GaugeWright can take a rail-contingent application fee when billing runs
       through the settlement plane. Attested runs also carry a metered compute
       floor (cost + margin) that is billed to the expert/consultant by default.

    Billing is policy layered over the system's safety machinery; it is **never**
    run or access authority — paying for something does not by itself grant the
    right to deploy or run it. The Stripe settlement rails have been verified in
    test mode, but production go-live, hosted infrastructure, enterprise packaging,
    and commercial licensing are still <span class="status planned">Planned</span>.
    See the [roadmap](reference/status.md).

    !!! note "Today"
        Only the local single-party workbench is shippable and free today. The repo
        is not open source yet (`license = "UNLICENSED"`), enterprise governance is
        built but not operationally live, and the paid managed services are not live
        production products yet.

## Deploying agents

??? question "Can I deploy an agent to a client today?"
    Not operationally yet. Cross-party [packaging](concepts/glossary.md#package)
    and deployment are **implemented and tested in the core**, but live deployment
    to a remote client is not yet wired up.
    <span class="status built">Built</span>
    <span class="status planned">live Planned</span>

    What you **can** do today:

    1. **Build** an [archetype](concepts/glossary.md#archetype) on the local
       workbench and refine it in an *edit chat*.
       <span class="status available">Available</span>
    2. **Run and review** it locally — each [run](concepts/glossary.md#run) works
       in an isolated sandbox and returns a diff you keep or discard.
       <span class="status available">Available</span>

    What is **Built but not yet operationally available** to end-users:

    - **Collaborate across machines** with another party via
      [federation](concepts/glossary.md#federation) (certificate-pinned TLS, a
      relay that routes only encrypted bytes). This is implemented and exercised
      only in a loopback + NAT-isolated CI harness — it is not a shippable
      cross-machine path today.
      <span class="status built">Built</span>

    See [Package &amp; deploy](guides/expert/package-and-deploy.md) and
    [Deployment modes](concepts/deployment-modes.md) for the full picture, and the
    [roadmap](reference/status.md) for what goes live next.

??? question "Can a deployed agent leak my method, or escape its sandbox?"
    The protections are **structural** — built into how runs work, not a setting
    you can misconfigure:

    - A run can only do what it was **admitted** to do — no ambient power to read,
      call, retain, reveal, or export beyond the work it was handed.
      <span class="status available">Available</span>
    - A running agent **cannot rewrite its own method**: the agent definition is
      editable only from an *edit chat*, never a *work chat*, enforced by an OS
      sandbox at the kernel — so even a shell inside a run can't change it.
      <span class="status available">Available (Linux/macOS)</span>
    - On **Windows**, the kernel method-isolation sandbox is not built yet.
      <span class="status planned">Planned</span>

    These are paired with adversarial tests that fail if the protection is
    removed. See [How GaugeWright protects your work](concepts/protection.md).

## Certs, signing &amp; source

??? question "Why are the downloads unsigned?"
    Code-signing and notarization are being set up. Until then, use the OS
    override:

    === "macOS"
        Right-click the app → **Open**.

    === "Windows"
        At the SmartScreen prompt, choose **More info → Run anyway**.

    === "Linux"
        Mark the `.AppImage` executable, or install the `.deb`.

    All releases are on
    [GitHub](https://github.com/jamesjscully/gaugedesk/releases). Code-signing is a
    known gap. <span class="status planned">Planned</span>

??? question "Is GaugeWright SOC 2 / ISO 27001 certified?"
    Not yet. SOC 2 Type II, a DPA with a published subprocessor list, and an
    independent penetration test are committed and prioritized but not yet
    available. <span class="status planned">Planned</span>

    Distinguish two kinds of assurance:

    - **Structural guarantees** — the confidentiality and isolation invariants are
      stated formally and **machine-checked** in the codebase today (e.g. a relay
      cannot read payload; a handle is not access).
      <span class="status available">Available</span>
    - **Policy / operational assurance** — third-party audits, attestations, and a
      published compliance posture. <span class="status planned">Planned</span>

    See the [architecture &amp; security documentation](security.md) for the
    invariant→control crosswalk and an honest compliance posture.

??? question "Where's the source?"
    On [GitHub](https://github.com/jamesjscully/gaugedesk). The authoritative
    specification and the formal models live there, and this documentation lives
    alongside the code.

## Where to go next

- New here: **[Getting started](getting-started.md)** — download to first run.
- Learn the model: **[Concepts](concepts/index.md)** ·
  **[Glossary](concepts/glossary.md)**.
- Understand the safety model: **[How GaugeWright protects your work](concepts/protection.md)**.
- What's live vs coming: **[Roadmap &amp; status](reference/status.md)**.
- Build and ship as an expert: **[Build an agent](guides/expert/build-an-agent.md)** ·
  **[Package &amp; deploy](guides/expert/package-and-deploy.md)**.
