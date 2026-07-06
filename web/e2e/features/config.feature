Feature: Archetype settings

  As a user I can edit an archetype's config (the method its chats — and every
  placement of it — inherit, ADR 0035); a valid config is parsed-then-saved, and
  a malformed one is rejected at the boundary (never written).

  Scenario: save a valid config
    Given the workbench is open
    When I open the config editor
    And I set the config to "{\"model\":\"gpt-5.5\",\"policy\":{\"posture\":\"trust-by-default\"}}"
    Then the config status shows "saved"

  Scenario: reject a malformed config with a plain-language message
    Given the workbench is open
    When I open the config editor
    And I set the config to "{ not valid json"
    Then the config status shows "isn't valid"
