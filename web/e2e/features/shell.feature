Feature: The workbench shell

  As a user I see the four-panel workbench with a project-first facet browser,
  so I can orient myself across projects, the library of archetypes, and every
  chat (ADR 0035/0036).

  Scenario: the facet browser pivots by Chats, Projects, and Library
    Given the workbench is open
    Then the facet "Chats" is active
    And the facet "Projects" is present
    And the facet "Library" is present

  Scenario: the search box filters across the facet (navigation.md B2)
    Given the workbench is open
    When I create an archetype named "Zephyr"
    Then I see the archetype "Zephyr"
    And I see the archetype "assistant"
    When I search the facets for "Zeph"
    Then I see the archetype "Zephyr"
    And the archetype "assistant" is hidden
    When I clear the facet search
    Then I see the archetype "assistant"

  Scenario: the panels are labelled Chat and Files
    Given the workbench is open
    Then the run pane is labelled "Chat"
    And the workspace pane is labelled "Files"

  Scenario: collapsing a project folds and unfolds its placements
    Given the workbench is open
    When I create a project named "collapsible"
    And I place an archetype on the project "collapsible"
    Then the project "collapsible" shows its placements
    When I collapse the project "collapsible"
    Then the project "collapsible" hides its placements
    When I collapse the project "collapsible"
    Then the project "collapsible" shows its placements

  Scenario: collapsing a placement folds and unfolds its chats
    Given the workbench is open
    When I create a project named "placefold"
    And I place an archetype on the project "placefold"
    And I add a work chat in project "placefold"
    Then the placement in project "placefold" shows a chat
    When I collapse the placement in project "placefold"
    Then the placement in project "placefold" hides its chats
    When I collapse the placement in project "placefold"
    Then the placement in project "placefold" shows a chat
