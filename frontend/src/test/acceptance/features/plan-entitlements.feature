# Executable spec: src/test/acceptance/plan-entitlements.test.tsx
# Demo clock is frozen at 2026-07-14T15:30Z; data comes from the seeded demo tenants.
Feature: Plan entitlements
  Workspace plans (Free, Starter, Business, Enterprise) cap seats, projects and
  storage, and price the subscription per occupied seat. The billing page must
  surface those caps truthfully for the active tenant and never render broken
  pricing.

  Background:
    Given the seeded workspace "Nimbus Co" (w3) on the Free plan
    And Nimbus Co has 3 seat holders (1 owner, 2 workers)

  Scenario: Free plan seat ceiling
    When the owner opens the billing page
    Then the usage panel says limits come from the Free plan
    And the Seats meter reads 3 of 3 members
    And the meter is full at 100% and drawn in the warning colour
    And the plan is priced at $0 per user across 3 seats with no broken math

  Scenario: Plan upgrade lifts limits
    Given Nimbus Co sits at its Free seat ceiling
    When the workspace is upgraded to the Business plan
    Then the usage panel says limits come from the Business plan
    And the Seats meter reads 3 of 50 members
    And the meter drops to 6% so headroom is visible
    And the old 3-seat ceiling is gone

  Scenario: Enterprise custom pricing
    When the workspace moves to the Enterprise plan
    Then the monthly cost shows "Custom" instead of a number
    And the price line invites the owner to contact sales
    And no "/mo" suffix dangles next to a missing price
    And the seat limit reads Unlimited
    And "$NaN" never appears anywhere on the page
