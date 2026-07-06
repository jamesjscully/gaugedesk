Feature: Round 4 — config-edit safety, legible diffs, and one canonical chat title

  The round-4 review found that the assistant's settings file could be saved as
  garbage JSON with no validation, the split diff was illegible at the panel's
  width, and the TASKS bar still leaked the raw "new chat" placeholder. These
  scenarios lock in the fixes.

  Scenario: editing the assistant's settings file rejects invalid JSON with a plain message
    Given a new engagement
    When I task the agent with "make a note"
    Then the run phase is "Completed"
    When I reveal the internal files
    And I select the file ".agent-config.json" in the workspace
    And I open the "edit" tab
    Then the editor warns that this is the assistant's settings file
    When I replace the editor content with "{ this is not valid json"
    And I try to save the file
    Then the save is rejected with a plain-language message

  Scenario: a valid edit to the settings file still saves
    Given a new engagement
    When I task the agent with "make a note"
    Then the run phase is "Completed"
    When I reveal the internal files
    And I select the file ".agent-config.json" in the workspace
    And I open the "edit" tab
    And I replace the editor content with "{}"
    And I save the file

  Scenario: the split diff toggle is hidden when the review panel is too narrow
    Given a new engagement
    When I task the agent with "make a change"
    Then the run phase is "Completed"
    When I open the "diff" tab
    Then the split diff toggle is not offered at the default panel width

  Scenario: a finished turn surfaces in the task bar without the raw "new chat" placeholder
    Given a new engagement
    When I task the agent with "make a change"
    Then the task bar shows a review
    And the task bar shows no chat literally titled "new chat"
