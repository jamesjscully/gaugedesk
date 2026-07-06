Feature: Review shelf

  As a user I gate outputs behind conjunctive consent: review clears only once
  every required stakeholder consents, and only then can it be released.

  Scenario: conjunctive consent clears, then release
    Given a new engagement
    When I open the review shelf
    And I propose review
    Then the review phase is "Proposed"
    When the stakeholder "A" consents to review
    Then the review phase is "Proposed"
    When the stakeholder "B" consents to review
    Then the review phase is "Cleared"
    When I release the review
    Then the review phase is "Released"
