Feature: Billing
  A subscription is invoiced once a month for each occupied seat at the plan's per-user price.
  Free plans are never billed, at least one seat is always charged, and a second sweep in the same
  month never double-bills.

  Scenario: A paid plan is billed per occupied seat, once a month
    Given the workspace has a "Starter" subscription
    And 2 people occupy seats
    When the monthly invoice is generated
    Then an invoice for 12.00 is created
    And regenerating in the same month does not bill again

  Scenario: Free plans are never invoiced
    Given the workspace has a "Free" subscription
    When the monthly invoice is generated
    Then no invoice is created

  Scenario: At least one seat is always billed
    Given the workspace has a "Business" subscription
    When the monthly invoice is generated
    Then an invoice for 12.00 is created
