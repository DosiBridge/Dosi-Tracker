# Executable spec: src/test/acceptance/host-billing-lifecycle.test.tsx
# Endpoints: contract §7 (host additions) and the platform plan endpoints.
Feature: Host billing lifecycle
  The platform operator moves a paying customer through its subscription:
  changes its plan, activates it once paid (or to end a trial), records paid
  invoices, and keeps the plan catalogue current. Every change that affects
  what a customer is billed asks first and says what it does.

  Scenario: Change a tenant's plan
    Given the host on the tenants page of a live platform
    When they open a tenant, choose another plan and confirm
    Then the platform moves the tenant to that plan

  Scenario: Activate a subscription
    Given the host on a tenant's details
    When they choose Activate and confirm
    Then the tenant's subscription becomes active

  Scenario: See each tenant's plan and status
    Given a server that reports each tenant's subscription
    Then the tenants table and the tenant's details show its real plan, status and seats
    And the host can suspend an active tenant, or reactivate a suspended one, after confirming
    But against an older server plan and status read "—", never a guessed "Active"

  Scenario: Record a paid invoice
    Given a pending invoice on Platform Billing
    When the host marks it paid and confirms
    Then the invoice is recorded as paid

  Scenario: Export invoices safely
    Given an invoice for a tenant named like a spreadsheet formula
    When the host exports the invoices as CSV
    Then the tenant name is written as text, never as a formula

  Scenario: Keep the plan catalogue current
    Given the host on the plans page
    When they create a plan, edit one, or delete one without tenants
    Then the platform's plans change accordingly
    And a plan that still has tenants can't be deleted

  Scenario: Invoices of a deleted workspace
    Given an invoice whose tenant no longer has a name
    Then it reads as a deleted workspace, never as the host's

  Scenario: Deleting a tenant says what really happens
    Given the host on a live tenant's details
    Then the danger zone says sign-ins stop and that the data is not erased
