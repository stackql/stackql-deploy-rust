# Resource Processing Flows

This document describes every code path in the `build`, `test`, and `teardown` commands based on which anchors are present in a resource's `.iql` file.

## Anchor Reference

| Anchor | Purpose |
|--------|---------|
| `exists` | Check if resource exists (returns `count` or a named field) |
| `statecheck` | Verify resource properties match desired state |
| `create` | Create the resource |
| `update` | Update the resource (patch document) |
| `createorupdate` | Always execute (skip exists/statecheck) |
| `exports` | Extract values for downstream resources |
| `delete` | Remove the resource |

## Exists Query Variants

The `exists` query has two modes based on the returned column name:

**Count mode** — returns `count`, existence is `count > 0`:
```sql
SELECT count(*) as count FROM awscc.s3.buckets WHERE Identifier = '{{ bucket_name }}' AND region = '{{ region }}';
```

**Field capture mode** — returns a named field (e.g. `vpc_id`), captured as `this.<field>` for downstream queries:
```sql
SELECT split_part(ResourceARN, '/', 2) as vpc_id FROM awscc.tagging.tagged_resources WHERE ...;
```

When `null` or empty is returned in field capture mode, the resource is treated as non-existent.

---

## Build Flows

### Flow A: `createorupdate` + `exports`

Used for resources that should always be applied (e.g. routes, gateway attachments).

```mermaid
graph LR
    A[createorupdate] --> B[exports proxy?]
    B -->|rows returned| C[done ✓]
    B -->|no rows| D[FAIL ✗]
```

**Anchors:** `createorupdate`, optionally `exports`
**Examples:** `example_inet_route`, `example_inet_gw_attachment` (old pattern)

---

### Flow B: `exists`(count) + `statecheck` + `create`/`update` + `exports`

Classic pattern for resources identified by a known name/identifier.

```mermaid
graph LR
    A[exists count] -->|0| B[create]
    A -->|>0| C[statecheck]
    C -->|pass| D[exports]
    C -->|fail| E[update]
    B --> F[post-create exists]
    F --> G[statecheck]
    G --> D
    E --> H[post-update exists]
    H --> I[statecheck]
    I --> D
    D --> J[done ✓]
```

**Anchors:** `exists`(count), `statecheck`, `create`, optionally `update`, optionally `exports`
**Examples:** `databricks_account_credentials`, `aws_s3_workspace_bucket_policy`

---

### Flow C: `exists`(field) + `statecheck` + `create` + `exports`

Pattern for awscc resources using tagging for identification. The exists query returns a resource-specific field (e.g. `vpc_id`) which is captured as `this.<field>` and used in statecheck and exports.

```mermaid
graph LR
    A[exists field] -->|null| B[create]
    A -->|value| C["this.field = value"]
    C --> D["statecheck(this.field)"]
    D -->|pass| E["exports(this.field)"]
    D -->|fail| F["update(this.field)"]
    B --> G[post-create exists]
    G --> H["this.field = value"]
    H --> I["statecheck(this.field)"]
    I --> E
    F --> J[post-update exists]
    J --> K["statecheck(this.field)"]
    K --> E
    E --> L[done ✓]
```

**Anchors:** `exists`(field), `statecheck`, `create`, optionally `update`, `exports`
**Examples:** `example_vpc`, `example_subnet`, `example_inet_gateway`, `example_route_table`, `example_security_group`, `example_web_server`

---

### Flow D: `exists`(count) + `exports`-as-proxy (no statecheck)

The exports query doubles as a statecheck — if it returns rows, the resource is in the desired state.

```mermaid
graph LR
    A[exists count] -->|0| B[create]
    A -->|>0| C[exports proxy]
    C -->|rows returned| D[done ✓]
    C -->|no rows| E[update]
    B --> F[post-create exists]
    F --> G[exports proxy]
    G --> D
    E --> H[post-update exists]
    H --> I[exports proxy]
    I --> D
```

**Anchors:** `exists`(count), `create`, optionally `update`, `exports`
**Examples:** `aws_s3_workspace_bucket`, `databricks_storage_configuration`

---

### Flow E: `exists`(field) + `exports` (no statecheck)

Field capture from exists, exports uses `this.<field>`. Exports acts as statecheck proxy.

```mermaid
graph LR
    A[exists field] -->|null| B[create]
    A -->|value| C["this.field = value"]
    C --> D["exports proxy(this.field)"]
    D -->|rows returned| E[done ✓]
    D -->|no rows| F[update]
    B --> G[post-create exists]
    G --> H["this.field = value"]
    H --> I["exports proxy(this.field)"]
    I --> E
```

**Anchors:** `exists`(field), `create`, `exports`
**Examples:** `example_subnet_rt_assn`

---

### Flow F: `exists`(field) only (no statecheck, no exports)

Minimal pattern — exists confirms the resource, the captured field is the only output.

```mermaid
graph LR
    A[exists field] -->|null| B[create]
    A -->|value| C[done ✓]
    B --> D[post-create exists]
    D -->|value| C
```

**Anchors:** `exists`(field), `create`
**Examples:** Simple resources where existence is sufficient

---

## Test Flows

Test runs the same exists → statecheck/exports-proxy sequence as build, but never creates or updates. If a resource doesn't exist or fails statecheck, the test fails.

```mermaid
graph LR
    A[exists] -->|not found| B[FAIL ✗]
    A -->|found| C[statecheck or exports proxy]
    C -->|pass| D[exports]
    C -->|fail| B
    D --> E[done ✓]
```

---

## Teardown Flows

Teardown processes resources in reverse manifest order. First collects all exports (running exists to capture `this.*` fields), then deletes.

```mermaid
graph LR
    A[collect exports phase] --> B[exists per resource]
    B --> C[exports per resource]
    C --> D[reverse order delete phase]
    D --> E[exists check]
    E -->|found| F[delete]
    E -->|not found| G[skip]
    F --> H[post-delete exists]
    H -->|gone| I[done ✓]
    H -->|still there| I
```

During teardown export collection, exports that cannot be collected are set to `<unknown>` rather than failing - the stack may be partially deployed. An export ends up `<unknown>` when its query returns no rows, returns a non-fatal provider error, cannot be rendered, belongs to a `script` resource (never executed on teardown), or belongs to a resource with `skip_on_delete: true`.

Any teardown query (`exists`, `statecheck`, `exports`, `delete`, or inline `sql`) whose rendered text contains `<unknown>` is skipped rather than executed. Running it would at best match nothing and at worst put the placeholder into a hostname or identifier (for example `https://<unknown>.cloud.databricks.com/...`), which fails with a fatal `dial tcp` error and aborts the whole teardown. The resource is logged as skipped and processing continues with the next one.

After each `delete` (and its `callback:delete`, if any) the `exists` query is polled until the resource is gone, up to `postdelete_retries` times `postdelete_retry_delay` seconds apart (options on the `exists` anchor, defaults 10 and 5). Only then is the delete re-issued, up to the `delete` anchor's `retries`.

A `delete` statement the provider rejects aborts the teardown by default (`--on-failure error`). With `--on-failure ignore` the failure is logged, the resource is reported as not confirmed deleted, and the next resource is processed; fatal network, auth and planner errors abort in both modes. Every teardown ends with a summary of resources whose delete could not be confirmed.

`skip_on_delete: true` opts a resource out of teardown explicitly:

| Type | Effect on teardown |
|------|--------------------|
| `query` | Query not executed; declared exports set to `<unknown>` |
| `resource` / `multi` | Exports still collected (downstream deletes may need them); `delete` not executed, resource retained |
| `command` / `script` | No change (never executed on teardown) |

---

## `this.*` Variable Lifecycle

When an exists query returns a named field (not `count`), the value is captured as a resource-scoped variable:

| Phase | Variable | Available to |
|-------|----------|-------------|
| exists returns `vpc_id` | `this.vpc_id` → `example_vpc.vpc_id` | statecheck, exports, delete for this resource |
| exports returns `vpc_id` | `vpc_id` and `example_vpc.vpc_id` | all subsequent resources |

The `this.*` prefix is syntactic sugar — `{{ this.vpc_id }}` in `example_vpc`'s queries is preprocessed to `{{ example_vpc.vpc_id }}`.

---

## RETURNING * Optimization

When a `create` query includes `RETURNING *`, the Cloud Control API returns the operation metadata immediately. If `return_vals` is configured in the manifest, specified fields are captured as `this.*` variables, eliminating the need for a post-create exists re-run.

```yaml
resources:
  - name: example_vpc
    return_vals:
      create:
        - Identifier: vpc_id    # rename Identifier → this.vpc_id
        - ErrorCode             # capture as this.ErrorCode
```
