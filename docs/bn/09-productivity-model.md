# অধ্যায় ৯: The Productivity Model

> **Status:** Implemented (demo — `tenant-data.ts`, `reports-data.ts`, `monitor-data.ts`).  
> **As-built:** Client-side derivation only.

> **Canonical:** 0–100 score, categories, day split, team focus %।  
> Domain fields → [০৮](./08-domain-data-model.md) · Reports UI → [১০](./10-tenant-features.md)

---

## Productivity score

প্রতিটি activity/segment-এ **productivity (0–100)**।  
Apps তিন ভাগে (`AppCategory`):

| Category | উদাহরণ | UI color |
|----------|--------|----------|
| **Productive** | VS Code, Terminal, Figma, GitHub, docs | success-leaning |
| **Neutral** | Slack, email, calendar, Notion | muted |
| **Unproductive** | YouTube, X, Reddit, Spotify | warning/danger |

Browser sites — domain-level category in `monitor-data.ts`।  
Colors: `categoryColor` in `reports-data.ts`।

---

## Member day split (`deriveSplit`)

Input: `total = trackedToday` (minutes), `p = productivity` (0–100)

```
productive   = total × p / 100
remaining    = total − productive
unproductive = remaining × 0.35
neutral      = remaining − unproductive
idle         = total × 0.08
```

ব্যবহার: Dashboard donut, Insights cards, Apps report, Host avg productivity।

---

## Team focus %

```
focus% = productive ÷ (productive + neutral + unproductive)
```

Idle আলাদা দেখানো হয়; focus% denominator-এ idle নয় (demo formula)।

---

## Monitor browser rollup

Browser `DaySegment`-এ:

- Inline **Open tabs** list  
- Per-day **Browser activity** card: browsing time, distinct sites, tabs opened, category bar, per-site table  

“কোন সাইটে কত সময়” — Apps report-এর সাথে consistent categorization।

---

## কোথায় দেখা যায়

| Surface | কীভাবে ব্যবহার করে |
|---------|-------------------|
| Admin dashboard | Avg productivity, hourly focus |
| Monitor | Segment rings + browser cards |
| Reports → Productivity / Apps | Split + ranking |
| Insights | Top performer, distraction time |
| Host overview | Member-weighted avg productivity |

---

## Production notes

1. Score **server-side** recompute — client cheat এড়ানো  
2. Category list tenant-configurable (enterprise) — [১১](./11-platform-settings.md)  
3. Privacy: score ≠ keystroke content — [০১](./01-product-vision.md)  
4. Payroll report rates ≠ productivity score (আলাদা field)
