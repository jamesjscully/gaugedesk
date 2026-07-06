Feature: The archetype & project library

  As a user I browse and manage archetypes (the Library of methods), projects,
  placements, and chats from the project-first facet browser (ADR 0035/0036) —
  created via affordances, edited via right-click context menus.

  Scenario: a fresh workbench seeds a default archetype
    Given the workbench is open
    When I switch to the "Library" facet
    Then I see the archetype "assistant"

  Scenario: create an archetype and open an edit chat under it
    Given the workbench is open
    When I create an archetype named "reviewer"
    Then I see the archetype "reviewer"
    When I add an edit chat under the archetype "reviewer"
    Then the run phase is "Init"

  Scenario: a project places an archetype and hosts a work chat
    Given the workbench is open
    When I create a project named "client-site"
    Then I see the project "client-site"
    When I place an archetype on the project "client-site"
    And I add a chat under the placement
    Then the run phase is "Init"

  Scenario: one archetype placed on two projects (the many-to-many relation, C1)
    Given the workbench is open
    When I create a project named "alpha-site"
    And I place an archetype on the project "alpha-site"
    And I create a project named "beta-site"
    And I place an archetype on the project "beta-site"
    Then the project "alpha-site" shows its placements
    And the project "beta-site" shows its placements
    And the Library lists 1 archetype

  Scenario: delete an archetype via its context menu
    Given the workbench is open
    When I create an archetype named "scratch"
    Then I see the archetype "scratch"
    When I delete the archetype "scratch"
    Then the archetype "scratch" is gone

  # WS-H: a workstream can be started from any chat row, in the Chats facet, with no
  # root-picking — the placement resolves to the chat's own home and the chat joins
  # immediately, so the new shared line is a visible, non-empty group from the start.
  Scenario: create a workstream from a chat in the Chats facet
    Given the workbench is open
    When I start a new chat from All chats
    Then I see a chat in All chats
    When I create a workstream named "sprint" from that chat
    Then the chat is on the workstream "sprint"

  # Leaving empties the line but does not delete it — the line stays visible (and
  # joinable) with an empty member list, no hint message (WS-H).
  Scenario: leaving a workstream empties it but keeps it visible with a hint
    Given the workbench is open
    When I start a new chat from All chats
    And I create a workstream named "sprint" from that chat
    Then the chat is on the workstream "sprint"
    When I remove that chat from its workstream
    Then the workstream "sprint" shows it has no chats yet

  # Archiving closes the line for good: it disappears from the nav (only active lines
  # group chats) and its chat returns to the mainline list (WS-F INV-23 rehoming).
  Scenario: archiving a workstream removes the line and frees its chat
    Given the workbench is open
    When I start a new chat from All chats
    And I create a workstream named "sprint" from that chat
    Then the chat is on the workstream "sprint"
    When I archive the workstream "sprint"
    Then there is no workstream "sprint"
    And I see a chat in All chats

  # Promote brings the line's settled work into the placement mainline (explicit);
  # the line itself survives the promote (it is not archived) and keeps its chat.
  Scenario: promoting a workstream lands its work and keeps the line
    Given the workbench is open
    When I start a new chat from All chats
    And I create a workstream named "sprint" from that chat
    Then the chat is on the workstream "sprint"
    When I promote the workstream "sprint"
    Then the chat is on the workstream "sprint"

  # A second chat in the same placement joins the existing line — from the Chats facet,
  # which now offers co-located lines as join targets (WS-H).
  Scenario: a second chat joins an existing workstream from the Chats facet
    Given the workbench is open
    When I start a new chat from All chats
    And I create a workstream named "sprint" from that chat
    Then the chat is on the workstream "sprint"
    When I start a new chat from All chats
    And I add the latest chat to the workstream "sprint"
    Then the workstream "sprint" has 2 chats

  # Per-placement config-only customization (placement.md): tweak a method for one
  # project/client without forking — a config overlay + notes on the placement, applied
  # to new chats there; the shared archetype is untouched.
  Scenario: customize a placement for one project without forking
    Given the workbench is open
    When I create a project named "AcmeCo"
    And I place an archetype on the project "AcmeCo"
    And I customize the placement in project "AcmeCo" with notes "AcmeCo prefers terse output"
    Then the placement in project "AcmeCo" shows it is customized

  # Fork lineage (ADR 0038): a fork shares its source's git history, shows the lineage,
  # and can pull the source's improvements down via a real 3-way merge.
  Scenario: a fork shows its source and can pull updates
    Given the workbench is open
    When I create an archetype named "base"
    And I fork the archetype "base"
    Then an archetype is forked from "base"
    When I pull updates into the fork of "base"
    Then an archetype is forked from "base"
