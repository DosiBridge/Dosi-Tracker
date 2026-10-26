---
title: Installing the desktop agent
keywords: install, agent, download, setup, windows, macos, linux, mac, update, upgrade
---
Dosi-Tracker tracks time through a small desktop agent that runs on Windows, macOS or Linux. To get started, open the Download page from the sidebar and pick the installer for your operating system. Windows uses a standard setup file; macOS uses a zip you drag into Applications; Linux offers a tarball, a .deb and an .rpm.

After installing, open the agent and sign in with the same email and password you use for the web dashboard. The agent asks you to choose a workspace if you belong to more than one, and then a project to track against. You can switch projects at any time from the agent's menu.

The agent needs you to grant it permission to read the active window title on macOS and Linux (Accessibility or similar), and it may ask Windows to allow it through the firewall for sign-in. None of this lets Dosi-Tracker see your screen; it only reads which app and window are focused, and only when you have turned that on.

If the agent stops tracking, check that it still shows you signed in and a project selected. A grey icon usually means it is paused; click it to resume. If sign-in fails repeatedly, your device may have been revoked by a workspace owner or manager — sign in again to get a fresh device id.

To update the agent, download the latest installer from the same Download page and run it over the existing install; your sign-in and settings are kept. The agent checks in with the server over HTTPS only; it never tracks on an unencrypted connection except to your own computer.

If you are moving to a new computer, just install the agent there and sign in; the old device stays listed under your account until you or an admin removes it.
