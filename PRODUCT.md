# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Backend engineers and coding agents investigating failures that are difficult to reproduce locally.

## Product Purpose

BugParcel captures a backend failure with its Git state, request state, fixtures, environment constraints, and failure contract so it can be replayed and independently verified.

## Positioning

The product makes an incident portable for an agent without letting the agent mutate the developer's source branch during reproduction or verification.

## Operating Context

The product runs locally alongside Git repositories, FastAPI services, Docker, and a local MCP server. The dashboard is a local operational surface for browsing and running those parcels.

## Capabilities and Constraints

The local MCP server can capture, inspect, reproduce, reduce, diagnose, propose a diff, verify it in a detached worktree, and clean up generated worktrees. Source branches are never changed by replay or verification.

## Brand Commitments

Use the BugParcel name and supplied logo. The dashboard is restrained, professional, and local-first; it must not borrow the marketing site's decorative treatment.

## Evidence on Hand

The repository includes a FastAPI sandbox and real captured checkout failure parcels in the configured local store.

## Product Principles

- Preserve the evidence before proposing a fix.
- Make the failure contract legible at a glance.
- Keep every risky operation isolated from the source branch.
- Prefer actionable operational detail to visual decoration.
