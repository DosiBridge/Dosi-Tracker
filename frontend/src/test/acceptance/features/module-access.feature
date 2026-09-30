# Executable spec: src/test/acceptance/module-access.test.tsx
# Role/route access matrix lives in src/lib/roles.ts (navAccess).
Feature: Role-gated module access
  A role only ever sees the modules it may use. Access is enforced at TWO
  points — the sidebar hides forbidden modules, and the RoleGuard route
  boundary blocks direct navigation. The two must always agree: a hidden
  module that is still reachable (or an offered one that is blocked) is a
  defect.

  Scenario: Client's restricted workspace view
    Given a signed-in client
    When the workspace sidebar renders
    Then only the Dashboard and Settings modules are offered
    And no monitoring, reporting, team or billing module is visible
    When the client navigates directly to /reports
    Then the "Restricted area" notice replaces the report content
    And the reduced sidebar has no accessibility violations

  Scenario: Worker cannot reach team management
    Given a signed-in worker
    When the workspace sidebar renders
    Then the Team module is missing while their own work modules remain
    When the worker navigates directly to /team
    Then the route guard blocks the page with the "Restricted area" notice

  Scenario Outline: Sidebar and route guard agree
    Given a signed-in <role>
    When sidebar visibility is compared with direct navigation for every module
    Then both enforcement points agree on every one of the nine modules

    Examples:
      | role   |
      | owner  |
      | admin  |
      | worker |
      | client |
