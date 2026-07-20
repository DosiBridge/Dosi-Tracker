# পরিশিষ্ট: Glossary (পরিভাষা)

> **Status:** Reference.  
> **English:** [`../../DOCUMENTATION.md`](../../DOCUMENTATION.md) section 23.  
> কোর্স সূচি → [`README.md`](./README.md)

এই ফাইল **অধ্যায় নম্বর নয়** — সারা কোর্সের শব্দভাণ্ডার। ডুপ্লিকেট টপিক এড়াতে শুধু সংজ্ঞা।

---

## প্ল্যাটফর্ম ও SaaS

| শব্দ | অর্থ |
|------|------|
| **Tenant / Workspace** | এক কোম্পানির isolated product instance |
| **Host** | SaaS operator — সব tenant চালায় |
| **Plan / Edition** | মূল্য স্তর + seats/storage/retention সীমা |
| **MRR / ARR** | Monthly / Annual Recurring Revenue |
| **Subscription status** | `trialing` / `active` / `past_due` (Suspended) |
| **Impersonation** | Host সাময়িকভাবে tenant owner হিসেবে দেখে |

---

## Roles

| শব্দ | অর্থ |
|------|------|
| **Owner** | Tenant subscription holder — billing access |
| **Admin** | Tenant manager — billing ছাড়া |
| **Worker / Member** | Tracked individual contributor |
| **Client** | Read-only external stakeholder |
| **Host (role)** | Platform operator role (`/host/*`) |

বিস্তারিত matrix → [০২](./02-personas-roles.md)।

---

## Domain

| শব্দ | অর্থ |
|------|------|
| **Activity** | Tracked work block (window, counts, screenshot, productivity) |
| **DaySegment** | Monitor-এর দিনের এক টুকরো (work/meeting/break/idle) |
| **BrowserTab** | Browser segment-এর open tab metadata |
| **Productivity score** | 0–100 focus score per activity/segment |
| **Productivity split** | productive / neutral / unproductive / idle |
| **client_activity_id** | Agent-side idempotency key for ingest |

Domain → [০৮](./08-domain-data-model.md) · Score → [০৯](./09-productivity-model.md)।

---

## Architecture & ops

| শব্দ | অর্থ |
|------|------|
| **Agent** | Native desktop capture process (Rust/Swift) |
| **Offline queue** | Local SQLite before sync |
| **OpenIddict** | ABP auth server (tokens) |
| **TenantId filter** | Shared-DB isolation column + ABP global filter |
| **Presigned URL** | Temporary private access to screenshot blob |
| **RPO / RTO** | Recovery Point / Time Objective (backup/DR) |
| **As-built** | কোডে যা আজ আছে — [`../AS_BUILT.md`](../AS_BUILT.md) |
| **Target design** | কোর্সের লক্ষ্য আর্কিটেকচার (এখনো নাও থাকতে পারে) |

---

## Status labels (ডক পড়ার সময়)

| Label | অর্থ |
|-------|------|
| **Implemented** | Repo-তে কাজ করে (stated scope) |
| **Partial** | কোড আছে, end-to-end নয় |
| **Scaffold** | Template / placeholder |
| **Mock** | Demo UI / localStorage |
| **Not present** | Repo-তে নেই |
| **Target** | কোর্স লক্ষ্য — ship claim নয় |
