Feature: The egress membrane blocks out-of-policy effects

  gaugewright's thesis is "external expertise, safely deployed". Every tool effect
  passes the membrane: an effect the archetype's policy forbids is stopped, while
  in-policy effects proceed — and the user sees both.

  Scenario: a policy-blocked tool is stopped at the membrane
    Given a new engagement whose archetype blocks bash
    When I task the agent with "do some work"
    Then the run phase is "Completed"
    And a mediated tool line is shown
    And the transcript shows "bash blocked by membrane"
