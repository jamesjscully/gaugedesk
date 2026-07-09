Feature: Output catalog (O-4)

  As a user I can see the deliverables this chat has produced and their review
  status, in the history shelf's Outputs tab — outside developer mode. The catalog
  reads the produced output resources from the GET /resources projection and shows
  each output's availability plus its review/export gating (INV-10: metadata only).

  Scenario: a produced output appears in the catalog with its review state
    Given a new engagement
    When I attach the context folder "/home/jack/code/gaugedesk/plugin"
    And I task the agent and let the turn settle
    When I open the outputs catalog
    Then the outputs catalog lists an output
    And the output shows its review state

  Scenario: the outputs catalog is empty before anything is produced
    Given a new engagement
    When I open the outputs catalog
    Then the outputs catalog shows no outputs

  Scenario: a held output shows its stakeholders and can be consented to release (UX-11)
    Given a new engagement
    When I attach the context folder "/home/jack/code/gaugedesk/plugin"
    And I task the agent and let the turn settle
    And review is proposed on this chat's output
    And I open the outputs catalog
    Then the held output shows stakeholder "local-user"
    When I consent to release the held output for "local-user"
    And I release the held output
    Then the held output is released
