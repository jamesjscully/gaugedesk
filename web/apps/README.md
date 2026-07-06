# Web App Split Shims

These directories are transitional entry points for the future frontend package
split. App roots own their moved implementation files; old `src/` paths remain
as compatibility stubs that import these apps through workspace package names
until publish metadata and lower-level helper/state ownership are settled.

Each app root has a private `package.json` with `gaugewright` metadata for the
future repo target, source posture, CI lane, dependency policy, and runtime
dependencies. These files are extraction markers in the root npm workspace;
dev/build dependencies still stay at the root, but each app has source-derived
subpath exports, a local `typecheck` script, plus an app-local `vite.config.ts`
and `build` script for one-app extraction rehearsals.

App code should import sibling roots through declared workspace package names
such as `@gaugewright/enterprise-client`, `@gaugewright/managed-client`, and
`@gaugewright/workbench-ui/styles.css`, not through `../../../packages/*/src`
or `../../../apps/*/src` paths.

Do not put new product logic here yet. Move code into these apps only after the
corresponding tsconfig/build lane is green.
