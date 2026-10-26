---
title: Projects and what gets captured
keywords: project, capture, screenshot, webcam, track, tracking, activity, productivity, idle, privacy, consent
---
A project is what time gets tracked against. An owner or manager with edit rights creates projects from the Projects page and adds members to each one, optionally as a client who only observes. The desktop agent lets the signed-in member pick one of their assigned projects before it starts a block of tracked time.

Each project has its own capture settings, set by whoever can edit it: whether screenshots are taken, how often, whether the webcam is sampled, and whether keyboard and mouse activity is measured at all. Activity is always a count of input events over time, never the content typed or clicked, and the agent never runs with elevated or root privileges to read it. Webcam capture is off by default everywhere and must be turned on deliberately for a project.

"Activity %" on reports means the share of tracked minutes that had some keyboard or mouse input; it says nothing about the quality or amount of work done, since plenty of real work (reading, calls, planning) involves little typing. If a project does not measure input at all, activity shows as "not measured" rather than a false zero.

A tracked block ends when the agent is closed, the computer sleeps or locks, the member switches projects, or after a period of no detected activity (idle timeout), whichever the project's settings define. Two blocks of the same member are not allowed to overlap by more than a few seconds, so clock drift between restarts does not duplicate time.

Screenshots and webcam captures, where enabled, are stored for the retention period the workspace owner has configured and can be deleted earlier under your workspace's privacy settings. None of this capture data, nor raw input counts, is ever sent to the AI assistant.
