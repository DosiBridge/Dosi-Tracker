---
title: Time zones and how days are counted
keywords: timezone, zone, dst, daylight, day, date, calendar, basis
---
Every report groups tracked time into local calendar days, using a time zone. By default this is your workspace's configured reporting zone, but an individual report can instead count each member's day in their own device's time zone, if the workspace has turned that "day basis" option on; ask a manager which your workspace uses if you are unsure.

A local day runs from midnight to midnight in whichever zone applies. When daylight saving time changes partway through a tracked block, the block is still assigned to the day it started in; the short or long day that results is normal and expected twice a year.

Weeks on Dosi-Tracker's reports run Monday to Sunday. "This week" always means the current Monday through today, not through Sunday, since days that have not happened yet are never reported.

If your tracked hours look off by exactly one hour, check whether your workspace recently changed its reporting zone, or whether a daylight-saving change happened inside the period you are viewing — both are common, harmless causes, not a tracking bug.

You can see which zone a report used in the report's own header or export; AI-written summaries always say the zone they used too, if you look at the detailed figures behind a summary.
