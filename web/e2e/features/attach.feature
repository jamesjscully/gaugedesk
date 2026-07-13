Feature: Message attachments

  The composer's paperclip clips file(s) to the message being composed: text files
  inline into the turn, images ride along as native WhippleScript resources (message-scoped,
  no workspace ingest). It is distinct from the durable Files-bar upload (context).

  Scenario: attach a text file to a message
    Given a new engagement
    When I attach the file "notes.txt" containing "hello from attachment"
    And I task the agent with "summarize the attachment"
    Then the transcript echoes my message "hello from attachment"
    And the composer has no pending attachments

  Scenario: attach an image to a message
    Given a new engagement
    When I attach a PNG image "screenshot.png"
    Then the composer shows an image attachment "screenshot.png"
    When I task the agent with "describe the screenshot"
    Then the run phase is "Completed"
    And the composer has no pending attachments

  Scenario: PDF and Office files are not supported yet
    Given a new engagement
    When I attach an unsupported file "report.pdf" of type "application/pdf"
    Then the composer has no pending attachments
