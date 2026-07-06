Feature: Content search reaches into the chat log (SEARCH-1)

  Search matches not only chat titles but the chat's transcript: a chat whose log
  mentions the query surfaces in the tree even when its title does not, carrying a
  snippet of the match (navigation.md "Search scope and relevance"). The title tier
  stays a client-side filter; the chat-log tier is the server's GET /search.

  Scenario: a word only in the chat log surfaces the chat with a snippet
    Given the workbench is open
    When I switch to the "Chats" facet
    And I start a new chat from All chats
    And I task the agent with "review the quarterly numbers"
    And I search the facets for "agent-note"
    Then a chat surfaces with a content snippet
