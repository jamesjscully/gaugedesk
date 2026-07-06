Feature: Cross-machine federation

  Two authorities on two control planes pair over the network and collaborate
  through the cert-pinned relay legs, driven from the workbench UI. Alice is the
  primary instance (port 7878, authority local-user); Bob is the peer (port 7879)
  reached by pointing a second browser window at it with ?cp=.

  # One scenario, one pairing: the rendezvous broker is shared and long-lived, so
  # a single pairing keeps its parked receiver legs unambiguous (re-pairing across
  # scenarios would leave stale legs on the reused session tokens).
  Scenario: pair two machines and collaborate both ways
    Given the two federated workbenches are open
    When the two authorities pair with each other
    And the owner crosses a handle to the peer
    Then the handle appears in the peer's federation inbox
    When the owner places a remote run on the peer
    Then the owner sees the peer's observations were admitted
    When the owner hands off a project's home
    Then the project's handoff is committed to the target
