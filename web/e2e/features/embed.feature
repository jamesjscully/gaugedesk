Feature: Embedded panels (EMBED-2)

  As a consultant I embed the workbench panels as web components on my own page,
  driven by a scoped session over the deployment's control plane — the same panels
  the desktop renders, mounted against a remote Session (EMBED-1's contract).

  Scenario: the embedded chat renders and sends against a scoped session
    Given the embed example page is open
    Then the embedded chat shows a composer
    When I send "hello from the embed" in the embedded chat
    Then the embedded transcript shows "hello from the embed"

  Scenario: the embedded panels carry the workbench theme and accept --gw-* overrides
    Given the embed example page is open
    Then the embedded chat is themed by the workbench palette
    And a "--gw-bg" override cascades into the panel's shadow root
