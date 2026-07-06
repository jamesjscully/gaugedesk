Feature: All-chats "just start typing" quick-start

  The All chats facet is rootless, so its "+ new chat" affordance roots the chat
  on the hidden Personal default placement (ADR 0036) — a work chat (ADR 0035)
  with no project or method setup. The "just start typing" entry point.

  Scenario: a new chat from All chats opens a work chat with no setup
    Given the workbench is open
    When I switch to the "Chats" facet
    And I start a new chat from All chats
    Then the run phase is "Init"
    And the active chat is a work chat
    And I see a chat in All chats
