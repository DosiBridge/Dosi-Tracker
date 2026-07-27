Feature: Activity tracking
  Tracked activity is attributed to the person tracking it and may only be recorded for a project
  they belong to. Re-uploading the same tracked block is idempotent, and activity lists newest-first.

  Scenario: A member records activity for their own project
    Given I have created a project called "Website"
    When I submit a tracked activity for the project
    Then the activity is recorded and attributed to me

  Scenario: Tracking a project I do not belong to is refused
    Given a project exists that I do not belong to
    When I submit a tracked activity for that other project
    Then tracking is refused as unauthorized

  Scenario: Re-uploading the same activity does not create a duplicate
    Given I have created a project called "Website"
    When I submit the same activity twice
    Then only one activity is stored for the project

  Scenario: Activity is listed newest first
    Given I have created a project called "Website"
    When I submit tracked activities starting at hours 8, 10 and 9
    Then the activities are listed newest first
