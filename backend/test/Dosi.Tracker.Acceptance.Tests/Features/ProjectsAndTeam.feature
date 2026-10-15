Feature: Projects and team management
  A workspace owner creates projects and builds a team on them. Membership is seat-limited by the
  plan, a person can only join a project once, and joining notifies them.

  Scenario: The creator of a project is enrolled as its admin
    Given I have created a project called "Website"
    When I list the members of the project
    Then I am a member with the role "Admin"

  Scenario: Adding a member enrols them and sends a notification
    Given I have created a project called "Website"
    When I add a "Worker" to the project
    Then the project has 2 members
    And the new member has been notified

  Scenario: A person cannot be added to the same project twice
    Given I have created a project called "Website"
    And I have added a worker to the project
    When I add the same worker again
    Then adding is rejected as a duplicate member

  Scenario: The plan seat limit is enforced
    Given the workspace is on the "Free" plan
    And I have created a project called "Website"
    And I have added 2 workers to the project
    When I add another worker
    Then adding is rejected because the seat limit is reached

  Scenario: Inviting an email without an account creates a pending invitation
    Given I have created a project called "Website"
    When I invite "new.worker@acme.test" as a "Worker"
    Then a pending invitation link is returned
    And no account exists yet for "new.worker@acme.test"
