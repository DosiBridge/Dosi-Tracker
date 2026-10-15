# Executable spec: src/test/acceptance/agent-onboarding.test.tsx
Feature: Connecting a desktop agent
  A new member installs the desktop agent and signs in. The agent can't guess
  which server its workspace lives on, so the web app tells them — and points
  them at the agent built for the computer they are on.

  Scenario: The Download page gives the sign-in details
    Given a signed-in member of a live workspace
    When they open the Download page
    Then it shows the server address to paste into the agent and their email (no workspace name is needed)
    And each can be copied

  Scenario: The agent for this computer is recommended
    Given a member browsing from a Mac
    When they open the Download page
    Then the macOS card is marked as the one for this computer

  Scenario: Every permission says what it is for
    Given the Download page
    Then the macOS steps name Screen Recording, Accessibility and Camera with what each is used for
    And the Windows steps name the installer and say no special permissions are needed

  Scenario: Members can always find the agent
    Given a signed-in member
    Then the sidebar and the command palette offer the Desktop agent page
