/**
 * Wire types of the backend. Every DTO, event payload and error type is GENERATED from the Rust
 * types (`crates/lampo-application/src/dto`, `events.rs`, `error.rs`) into `bindings.ts`:
 * never edit that file, change the Rust type and regenerate it with
 *
 *   cargo test -p lampo-application --lib ts_bindings -- --ignored
 *
 * (`cargo test` fails while it is out of date). Wire format: camelCase fields, kebab-case enum
 * strings. This file only adds front-end aliases.
 */
export type * from './bindings.js';

/** "php" | "apache" | "mod-fcgid" | "mysql" | "redis" | "composer" | "xdebug" | "php-redis" | … */
export type PackageKind = string;
