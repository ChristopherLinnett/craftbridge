# 00 — Brief

> The request exactly as given, then split into requirements. Nothing here is inferred: anything the brief leaves open becomes a question (`plan.mjs q add`), not an assumption.

## The request (verbatim)

> Ok, here's a new (hypothetical) type of project.. we want to build Swift/Kotlin plugins ready for a developer to use the engine of the different Artcraft apps.
>
> So we take the CLI commands, turn them into device APIs. Make sure the engine is connected appropriately as expected, eg. iOS is natively connected to what it uses as opposed to generic GPUs etc, and similar for android processes.
>
> Your task right now is to create a project folder, for artcraft-plugin-port and we'll start with effectcraft so artcraft-plugin-port/effectcraft
> then we'll place a plan folder. The plan is your stage, we may nest further for android/iOS variants. But the end goal is everything the CLI variant of effectcraft can do, can be done on Android/iOS plugins... It would be a bonus if we made it somehow linked to the original codebase.. not a direct link, but in a way where real updates to the base apps, would make it EASY to replicate the changes in our plugins if not deterministic.
>
> Right now the goal is a feasibility analysis, a plan, and if found feasible a /scoping-plan to be built to basically run the potential plan end to end.. ie. following the scoping plan to the letter would end up with the plugin ready to use in a android/iOS project for developers to connect their own interface up with the APIs exposed. You may launch a few subagents to help, you may do online research that helps, you may look in the base repo, and any recommendations that help scalability etc.. if there would be a way to have a universal plugin for generic parts of all the artcraft apps, with a specialised plugin that imports that universal plugin, then implements the app level features itself... This sort of thing.. but thats just something to assess, not a constraint

## Requirements

| ID | Requirement (in the user's terms) | Source line |
|---|---|---|
| RQ1 | Build Swift (iOS) and Kotlin (Android) plugins a developer can add to an app to use the engine of an ArtCraft app, starting with EffectCraft. | "we want to build Swift/Kotlin plugins ready for a developer to use the engine" |
| RQ2 | Turn the CLI commands into device APIs: everything the EffectCraft CLI can do can be done through the iOS and Android plugins. | "take the CLI commands, turn them into device APIs" · "everything the CLI variant of effectcraft can do, can be done on Android/iOS plugins" |
| RQ3 | The engine is connected to each platform's native facilities: on iOS what iOS natively uses rather than a generic GPU path, and the equivalent native processes on Android. | "iOS is natively connected to what it uses as opposed to generic GPUs etc, and similar for android processes" |
| RQ4 | The plugins stay linked to the original codebase without a direct link, so that real updates to the base app are easy, ideally deterministic, to replicate in the plugins. | "linked to the original codebase.. not a direct link … EASY to replicate the changes in our plugins if not deterministic" |
| RQ5 | The project lives at `artcraft-plugin-port/effectcraft`, with a `plan` folder; the layout may later nest per platform (android/iOS variants). | "create a project folder, for artcraft-plugin-port … artcraft-plugin-port/effectcraft … then we'll place a plan folder" |
| RQ6 | Deliver a feasibility analysis first. | "Right now the goal is a feasibility analysis" |
| RQ7 | If feasible, deliver a scoping plan that, followed to the letter, ends with plugins ready to use in an Android or iOS project, where developers connect their own interface to the exposed APIs. | "following the scoping plan to the letter would end up with the plugin ready to use … developers to connect their own interface up with the APIs exposed" |
| RQ8 | Assess a universal plugin for the generic parts of all ArtCraft apps, imported by a specialised per-app plugin that implements the app-level features; include recommendations that help scalability. This is an assessment, not a constraint. | "if there would be a way to have a universal plugin … just something to assess, not a constraint" |

## Constraints and audiences

- **Platforms:** iOS (Swift) and Android (Kotlin). EffectCraft first; other ArtCraft apps later.
- **Developer-facing:** the plugins expose APIs; the integrating developer supplies the user interface (RQ7).
- **Upstream codebase:** EffectCraft, `storytold/effectcraft`, Rust workspace (pinned in `plan.config.mjs → repos`). Its own rules bind any code that is contributed back to it: no `unsafe`, never crash, permissive licences only, no Adobe assets, ArtCraft brand marks unmodified and only in the context of EffectCraft (`AGENTS.md`).
- **Audiences:** the user and the team who will build the plugins (engineering), and app developers who will integrate them (the API reference and integration guide).
- **Status:** the user calls the project hypothetical; the plan is the stage at which go / no-go is decided.

## Not stated — raised as questions

Recorded with `plan.mjs q add` and asked in the first round (`plan.mjs q list`): owner (Q1), product naming under the ArtCraft brand licence (Q2), code home, licence and publishing (Q3), how the plugins stay linked to upstream (Q4), whether upstream seams may be contributed (Q5), which CLI capabilities ship (Q6), API shape (Q7), how frames reach the developer's interface (Q8). Research adds the platform questions.

## Sources

- The user's messages in the planning session of 2026-11-12.
