Feature: Your account (ACCT-1)

  The operator's own surface (ADR 0053), reached from Settings ▸ Your account: link an
  LLM provider account, see trusted devices, and settings. The linked token is sealed
  server-side (SEC-4) and never shown again.

  Scenario: link an AI provider account
    Given the workbench is open
    When I open my account
    And I link the "openai" account with token "sk-test-secret"
    Then "openai" shows as a linked account
