---
title: Privacy, data and account controls
keywords: privacy, data, delete, deletion, retention, export, erase, erasure, gdpr, account, keystroke, keylog
---
Dosi-Tracker measures keyboard and mouse activity only as counts of events over time; it never records which keys were pressed, never logs clipboard contents, and never captures text typed anywhere. There is no keylogging in this product, by design.

Each workspace owner sets a retention period for screenshots, webcam captures and tracked activity rows; a background process removes data older than that period automatically. Owners can also change or disable capture types (screenshots, webcam, activity measurement) per project at any time, and changes apply going forward, not retroactively to data already collected under the previous settings.

A member can ask for their own tracked activity in a workspace to be erased, subject to the workspace's policy; an owner can also arrange account-level deletion, including removing a person from every workspace they belong to. Exporting your own tracked data (as CSV or JSON) is available from the Reports or Activities page to anyone who can already see that data.

The desktop agent only ever runs with the same privileges as the person using the computer: it does not request administrator or root access, and it cannot see other users' sessions. It also never captures a locked screen.

AI assist (this assistant and the summary cards elsewhere) only ever reads data you could otherwise see through the dashboard and reporting pages, with people's names replaced by aliases before anything reaches the AI provider. Screenshots, webcam images, keystrokes and raw input counts are never sent to it, whatever your workspace's AI settings allow.
