# Executable spec: src/test/acceptance/host-operations.test.tsx
# Demo mode: with no backend token the host console renders the seeded tenants;
# live-mode fetches never fire in these scenarios.
Feature: Host platform operations
  The /host console belongs to the platform operator alone. The host oversees
  every tenant and may step into one as its owner (impersonation) — clearly
  announced and reversible. Tenant roles are locked out of the console.

  Scenario: Platform admin oversees tenants
    Given the signed-in host user
    When they open the host console
    Then the platform chrome renders instead of the lock screen
    And the platform overview links into tenant management
    When they review the tenants page in demo mode
    Then every seed workspace is listed (Dosi Labs, Acme Studio, Nimbus Co)

  Scenario: Host impersonates a tenant
    Given the host on the tenants page
    When they use "Login as tenant" on Acme Studio
    And the tenant dashboard loads from the persisted session
    Then a banner announces host impersonation of Acme Studio as Diego Alvarez
    And the tenant dashboard content is visible beneath it
    When they click "Return to Host"
    Then the impersonation banner disappears
    And they are sent back to the host console

  Scenario Outline: Tenant users cannot reach the host console
    Given a signed-in <role>
    When they browse to /host
    Then the "Host area" lock screen blocks them
    And it offers the login page instead

    Examples:
      | role   |
      | owner  |
      | admin  |
      | worker |
      | client |
