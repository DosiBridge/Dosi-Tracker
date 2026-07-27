Feature: Workspace signup
  A new customer self-registers a workspace. Registration provisions an isolated tenant,
  an administrator account, and a subscription on the chosen plan — trialing for paid plans,
  active immediately for Free.

  Scenario: Registering on a paid plan starts a trial
    When I register the workspace "acme" for admin "owner@acme.test" on the "Starter" plan
    Then the workspace is created with its own tenant
    And the tenant has an administrator "owner@acme.test"
    And the subscription is "trialing"

  Scenario: Registering on the Free plan activates immediately
    When I register the workspace "freeco" for admin "owner@freeco.test" on the "Free" plan
    Then the subscription is "active"

  Scenario: Registering on an unknown plan is rejected
    When I register the workspace "nope" for admin "owner@nope.test" on the "Platinum" plan
    Then registration is rejected because the plan does not exist
