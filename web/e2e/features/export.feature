Feature: Export gating

  Releasing a protected output past the boundary requires conjunctive consent
  from every source stakeholder AND the target's admission — export is the
  irreversible egress edge, so it stays gated until all parties agree.

  Scenario: export clears only after source consent and target admission
    Given a new engagement
    When I open the review shelf
    And I propose export
    Then the export phase is "Requested"
    When the source "A" consents to export
    And the source "B" consents to export
    Then the export phase is "Requested"
    When the target admits the export
    Then the export phase is "Cleared"
    When I export
    Then the export phase is "Exported"
