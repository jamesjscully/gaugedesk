@api
Feature: Package distribution (M2)
  Publishing, installing, and entitling agents across authorities. Driven over the
  control plane (HTTP), not the browser — these are backend mechanism behaviours
  with no UI surface yet (decision D-BDD-API: an HTTP-level BDD runner).

  Scenario: install grants no run authority until the deployment is entitled
    Given a published package "p1" at version "v1"
    When the target installs "p1"
    Then package "p1" shows status "Installed" in the catalog
    And a governed run of "p1" in "ctx" is not ready
    When "p1" is entitled for context "ctx"
    Then a governed run of "p1" in "ctx" is ready

  Scenario: withdrawal immediately drops availability and readiness
    Given a published package "p2" at version "v2"
    When the target installs "p2"
    And "p2" is entitled for context "ctx"
    Then a governed run of "p2" in "ctx" is ready
    When the source withdraws "p2"
    Then package "p2" shows status "Withdrawn" in the catalog
    And a governed run of "p2" in "ctx" is not ready

  Scenario: an unpublished package cannot be installed
    When the target installs "ghost"
    Then the install is rejected
