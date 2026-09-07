# Executable spec: src/test/acceptance/tenant-lifecycle.test.tsx
# Persistence mechanism: localStorage (dosi-workspaces-created / -patches / -deleted),
# read back by SessionProvider on mount — the "reload" steps remount a fresh tree.
Feature: Tenant lifecycle
  Owners can create, rebrand and delete workspaces (tenants). The workspace
  switcher chrome always reflects the current state, and the session survives
  a reload.

  Scenario: Owner creates a workspace
    Given the owner of the primary tenant "Dosi Labs"
    When they create "Rocket Corp" on the Starter plan through the switcher
    Then a confirmation says Rocket Corp is ready on the Starter plan
    And Rocket Corp becomes the active workspace with a "Starter · Trial" badge
    And Rocket Corp appears in the switcher list alongside the seed tenants
    When the app reloads
    Then Rocket Corp is still the active workspace on its Starter trial

  Scenario: Workspace rebrand
    Given "Acme Studio" is renamed to "Acme Collective" with a new brand colour
    When the owner opens the switcher
    Then the list shows "Acme Collective" and the old name is gone
    When the owner switches to it
    Then the chrome shows the new name and the avatar wears the new colour

  Scenario: Deleting the active workspace falls back to the primary tenant
    Given "Acme Studio" is the active workspace
    When the workspace is deleted
    Then the session falls back to the primary tenant "Dosi Labs"
    And Acme Studio no longer exists anywhere in the switcher
    And the other tenants survive untouched
