Feature: Admin console (M3)

  The enterprise org-facing surfaces (ADR 0043): organization, members, SSO, SCIM,
  security, billing, and the audit timeline. Post-split (SPLIT-2) the console is a
  **standalone enterprise app** (ee/web/apps/admin-console) pointed at a provisioned
  org control plane via `?cp=` — only the hosted composition mounts /admin/*, and the
  workbench bundle carries no admin surface at all (per DEPLOY-7, ADR 0059 §6, the
  solo collapse offers no org admin entry). On the loopback tenant these routes are
  ungated (single authority); the console is a thin renderer over /admin/*.

  Scenario: the org admin console is hidden in the solo collapse
    Given the workbench is open
    When I open the settings menu
    Then the organization admin entry is not offered

  Scenario: invite a member and see the action audited
    Given the admin console app is open for a provisioned tenant
    When I invite member "alice@acme.com" as "admin"
    Then the member "alice@acme.com" appears in the directory
    And the audit log shows the "member.invite" action

  Scenario: the admin console offers SSO sign-in
    Given the admin console app is open for a provisioned tenant
    Then the admin console offers SSO sign-in

  Scenario: the admin console shows the SP integration details
    Given the admin console app is open for a provisioned tenant
    Then the admin console shows the integration details

  Scenario: the guided SSO wizard walks through the steps
    Given the admin console app is open for a provisioned tenant
    When I launch the SSO setup wizard
    Then the SSO wizard shows the connect step
    When I advance the SSO wizard
    Then the SSO wizard shows the test step

  Scenario: the admin console shows the active-sessions roster
    Given the admin console app is open for a provisioned tenant
    Then the admin console shows the active sessions roster
