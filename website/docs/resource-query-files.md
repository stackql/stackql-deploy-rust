---
id: resource-query-files
title: Resource Query Files
hide_title: false
hide_table_of_contents: false
description: A quick overview of how to get started with StackQL Deploy, including basic concepts and the essential components of a deployment.
tags: []
draft: false
unlisted: false
---

import File from '/src/components/File';

Resource query files include the StackQL query templates to provision, de-provision, update and test resources in your stack.  Resource query files (`.iql` files) are located in the `resources` subdirectory of your project (stack) directory.  The `resources` section of the [`stackql_manifest.yml`](manifest-file) file is used to supply these templates with the correct values for a given environment at deploy time.

:::note

`.iql` is used as a file extension for StackQL query files by convention.  This convention originates from the original name for the StackQL project - InfraQL,  plus `.sql` was taken... 

:::

## Query types

A resource query file (`.iql` file) typically contains multiple StackQL queries.  Seperate queries are demarcated by query anchors (or hints), such as `/*+ create */` or `/*+ update */`.  These hints must be at the beginning of a line in the file, with the resepective query following on the subsequent lines.

:::tip

StackQL follows the ANSI standard for SQL with some custom extensions.  For more information on the StackQL grammar see the [StackQL docs](https://stackql.io/docs).

:::

The types of queries defined in resource files are detailed in the following sections.

### `exists`

`exists` queries are StackQL `SELECT` statements designed to test the existence of a resource by its designated identifier (does not test the desired state).  This is used to determine whether a `create` (`INSERT`) or `update` (`UPDATE`) is required.

An `exists` query can return results in one of two forms:

#### Count-based existence check

The query returns a single row with a single field named `count`.  A `count` value of `1` indicates that the resource exists, a value of `0` would indicate that the resource does not exist.

```sql
/*+ exists */
SELECT COUNT(*) as count FROM google.compute.networks
WHERE name = '{{ vpc_name }}'
AND project = '{{ project }}'
```

#### Identifier-based existence check

Alternatively, the query can return a field **other than** `count` (for example `identifier`).  If the query returns a row, the resource is considered to exist.  If no rows are returned, the resource does not exist.

Any non-`count` fields returned are automatically captured and made available as **resource-scoped variables** for all subsequent queries within the same resource (`statecheck`, `exports`, `delete`).  These captured fields are accessible using the `{{ this.<field_name> }}` syntax, which expands to `{{ <resource_name>.<field_name> }}`.

This pattern is particularly useful when you need to **discover a resource identifier** (for example, from a tag-based lookup) and then use that identifier to query the resource's actual properties in a `statecheck` or `exports` query.

```sql
/*+ exists */
WITH tagged_resources AS
(
    SELECT split_part(ResourceARN, '/', 2) as vpc_id
    FROM awscc.tagging.tagged_resources
    WHERE region = '{{ region }}'
    AND TagFilters = '{{ global_tags | to_aws_tag_filters }}'
    AND ResourceTypeFilters = '["ec2:vpc"]'
),
vpcs AS
(
    SELECT vpc_id
    FROM awscc.ec2.vpcs_list_only
    WHERE region = '{{ region }}'
)
SELECT r.vpc_id
FROM vpcs r
INNER JOIN tagged_resources tr
ON r.vpc_id = tr.vpc_id;
```

In the example above, when the resource exists the `vpc_id` field (e.g. `vpc-0abc123def456`) is captured and available as `{{ this.vpc_id }}` in subsequent queries:

```sql
/*+ statecheck, retries=5, retry_delay=5 */
SELECT COUNT(*) as count FROM
(
SELECT
AWS_POLICY_EQUAL(tags, '{{ vpc_tags }}') as test_tags
FROM awscc.ec2.vpcs
WHERE Identifier = '{{ this.vpc_id }}'
AND region = '{{ region }}'
AND cidr_block = '{{ vpc_cidr_block }}'
) t
WHERE test_tags = 1;
```

:::tip

The identifier capture pattern enables a powerful two-step workflow for providers like `awscc` (AWS Cloud Control) where resources are identified by tags rather than names:

1. **`exists`** — find the resource via a CTE that cross-references `awscc.tagging.tagged_resources` with the provider's `*_list_only` resource, capturing the cloud-assigned identifier.  The `INNER JOIN` ensures the resource both has the expected tags **and** currently exists (eliminating stale tag records for terminated resources).
2. **`statecheck`** — use `{{ this.<field> }}` to query the resource directly and verify its properties match the desired state (including tag comparison via `AWS_POLICY_EQUAL`).
3. **`exports`** — use `{{ this.<field> }}` to query the resource and extract values for downstream resources.

The [`to_aws_tag_filters`](template-filters#to_aws_tag_filters) filter converts the `global_tags` manifest variable into the AWS TagFilters format automatically.

:::

`preflight` is an alias for `exists` for backwards compatability, this will be deprecated in a future release.

### `create`

`create` queries are StackQL `INSERT` statements used to create resources that do not exist (in accordance with the `exists` query).

```sql
/*+ create */
INSERT INTO google.compute.networks
(
 project,
 name,
 autoCreateSubnetworks,
 routingConfig
) 
SELECT
'{{ project }}',
'{{ vpc_name }}',
false,
'{"routingMode": "REGIONAL"}'
```

### `createorupdate`

`createorupdate` queries can be StackQL `INSERT` or `UPDATE` statements, these queries are used for idempotent resources (as per the given provider if supported), for example:

```sql
/*+ createorupdate */
INSERT INTO azure.network.virtual_networks(
   virtualNetworkName,
   resourceGroupName, 
   subscriptionId, 
   location,
   properties,
   tags   
)
SELECT
   '{{ vnet_name }}',
   '{{ resource_group_name }}',
   '{{ subscription_id }}',
   '{{ location }}',
   '{"addressSpace": {"addressPrefixes":["{{ vnet_cidr }}"]}}',
   '{{ global_tags }}'
```

:::tip

You can usually identify idempotent resources using the `SHOW METHODS` command for a given resource, the the below example you can see a `create_or_update` method mapped to StackQL `INSERT`:

```plaintext {12}
stackql  >>show methods in azure.network.virtual_networks;
|-------------------------------|--------------------------------|---------|                                                                                                               
|          MethodName           |         RequiredParams         | SQLVerb |                                                                                                               
|-------------------------------|--------------------------------|---------|                                                                                                               
| get                           | resourceGroupName,             | SELECT  |                                                                                                               
|                               | subscriptionId,                |         |                                                                                                               
|                               | virtualNetworkName             |         |                                                                                                               
|-------------------------------|--------------------------------|---------|                                                                                                               
| list                          | resourceGroupName,             | SELECT  |                                                                                                               
|                               | subscriptionId                 |         |                                                                                                               
|-------------------------------|--------------------------------|---------|                                                                                                               
| create_or_update              | resourceGroupName,             | INSERT  |                                                                                                               
|                               | subscriptionId,                |         |                                                                                                               
|                               | virtualNetworkName             |         |                                                                                                               
|-------------------------------|--------------------------------|---------|                                                                                                               
| delete                        | resourceGroupName,             | DELETE  |                                                                                                               
|                               | subscriptionId,                |         |                                                                                                               
|                               | virtualNetworkName             |         |                                                                                                               
|-------------------------------|--------------------------------|---------|                                                                                                               
| check_ip_address_availability | ipAddress, resourceGroupName,  | EXEC    |                                                                                                               
|                               | subscriptionId,                |         |                                                                                                               
|                               | virtualNetworkName             |         |                                                                                                               
|-------------------------------|--------------------------------|---------| 
```

:::

`createorupdate` queries can also be used if a resource is updating the state of a pre-existing resource, for example:

```sql
/*+ createorupdate */
update aws.s3.buckets 
set PatchDocument = string('{{ {
    "NotificationConfiguration": transfer_notification_config
    } | generate_patch_document }}') 
WHERE 
region = '{{ region }}' 
AND Identifier = '{{ transfer_bucket_name }}';
```

### `delete`

`delete` queries are StackQL `DELETE` statements used to de-provision resources in `teardown` operations.

```sql
/*+ delete */
DELETE FROM google.compute.networks
WHERE network = '{{ vpc_name }}' AND project = '{{ project }}'
```

### `statecheck`

`statecheck` queries are StackQL `SELECT` statements designed to test the desired state of a resource in an environment.  Similar to `exists` queries, `statecheck` queries must return a single row with a single column named `count` with a value of `1` (the resource meets the desired state tests) or `0` (the resource is not in the desired state).  As `statecheck` queries are usually run after `create` or `update` queries, it may be necessary to retry the query to account for the time it takes for the resource to be created or updated by the provider.

```sql
/*+ statecheck, retries=5, retry_delay=10 */
SELECT COUNT(*) as count FROM google.compute.networks
WHERE name = '{{ vpc_name }}'
AND project = '{{ project }}'
AND autoCreateSubnetworks = false
AND JSON_EXTRACT(routingConfig, '$.routingMode') = 'REGIONAL'
```

:::tip

Useful functions for testing the desired state of a resource include [`JSON_EQUAL`](https://stackql.io/docs/language-spec/functions/json/json_equal), [`AWS_POLICY_EQUAL`](https://stackql.io/docs/language-spec/functions/json/aws_policy_equal), [`JSON_EXTRACT`](https://stackql.io/docs/language-spec/functions/json/json_extract) and [`JSON_EACH`](https://stackql.io/docs/language-spec/functions/json/json_equal).

:::

`postdeploy` is an alias for `statecheck` for backwards compatability, this will be deprecated in a future release.

### `exports`

`exports` queries are StackQL `SELECT` statements which export variables, typically used in subsequent (or dependant) resources.  Columns exported in `exports` queries need to be specified in the [`exports` section of the `stackql_manifest.yml` file](manifest-file#resourceexports).

```sql
/*+ exports */
SELECT
'{{ vpc_name }}' as vpc_name,
selfLink as vpc_link
FROM google.compute.networks
WHERE name = '{{ vpc_name }}'
AND project = '{{ project }}'
```

### `callback`

`callback` blocks are optional polling queries that run **after** a `create`, `update`, or `delete` DML statement to track the outcome of a long-running asynchronous operation.  They are only used when the preceding DML statement includes a `RETURNING *` clause that returns a tracking handle from the provider.

The canonical use-case is the AWS Cloud Control API (`awscc`), where every mutation returns a `ProgressEvent` object containing an `OperationStatus` and a `RequestToken` that must be polled until the operation completes.

```sql
/*+ callback:create, retries=10, retry_delay=15, short_circuit_field=ProgressEvent.OperationStatus, short_circuit_value=SUCCESS */
SELECT OperationStatus = 'SUCCESS' as success
FROM awscc.cloudcontrol.resource_request_statuses
WHERE region = '{{ region }}'
AND RequestToken = '{{ callback.ProgressEvent.RequestToken }}'
```

The operation qualifier (`:create`, `:update`, `:delete`) associates the callback with the matching DML anchor.  A plain `/*+ callback */` with no qualifier runs after **any** DML operation on the resource that used `RETURNING *`.

#### Callback options

| Option | Required | Default | Description |
|---|---|---|---|
| `retries` | no | 3 | Maximum number of poll attempts |
| `retry_delay` | no | 5 | Seconds to wait between attempts |
| `short_circuit_field` | no | — | Dot-path into the `RETURNING *` result checked before polling |
| `short_circuit_value` | no | — | Value of `short_circuit_field` that means polling can be skipped |

#### Success condition

The callback query must return a column named **`success`** (or `count`).  The operation is considered complete when the query returns a row where `success` equals `1` or `true`.  If the query returns no rows, or the value is not truthy, the runner waits `retry_delay` seconds and retries.  If all retries are exhausted the run fails with a timeout error.

#### RETURNING * capture and the `callback.*` namespace

When a DML statement includes `RETURNING *` the first row of the provider response is automatically captured and stored in the template context:

- **`callback.{field}`** — shorthand form, available within the current resource's own `.iql` file (overwritten by the next DML with `RETURNING *` on any resource).
- **`{resource_name}.callback.{field}`** — fully-qualified form, available to any downstream resource for the rest of the stack run.

For example, after a `create` on `aws_s3_workspace_bucket` that returns:

```json
{
  "ProgressEvent": {
    "OperationStatus": "SUCCESS",
    "RequestToken": "a0088b9e-db47-4507-b93e-345b77979626"
  }
}
```

The following keys become available:

```
callback.ProgressEvent.OperationStatus          → SUCCESS
callback.ProgressEvent.RequestToken             → a0088b9e-...
aws_s3_workspace_bucket.callback.ProgressEvent.OperationStatus → SUCCESS
aws_s3_workspace_bucket.callback.ProgressEvent.RequestToken    → a0088b9e-...
```

`RETURNING *` without a `callback` block is valid — the result is captured and no polling occurs.

#### Short-circuit

Some providers return the final operation status synchronously in the `RETURNING *` response.  When `short_circuit_field` and `short_circuit_value` are set, the runner checks the named field in the already-captured result **before** making any poll attempt.  If the value matches, the callback is skipped entirely.

#### Scope boundaries

- Callbacks only run during `build` and `teardown`.  The `test` command runs no DML and is unaffected.
- In dry-run mode, neither the `RETURNING *` capture nor the callback query executes.  Intent is logged instead.

#### Known limitations

- There is no mechanism to short-circuit retries on a terminal failure state (e.g. `OperationStatus = 'FAILED'`).  The runner retries until success or exhaustion.
- `RETURNING *` only captures the **first** row of the response.
- The `callback.*` shorthand is implicitly scoped to the current resource's `.iql` file.  Use the fully-qualified `{resource_name}.callback.*` form in downstream resources.

### `troubleshoot`

`troubleshoot` blocks are optional diagnostic queries that run automatically when a post-operation verification fails - specifically when a `build` statecheck fails or a `teardown` delete cannot be confirmed.  They provide visibility into why an operation failed by querying the provider for status information that is not available in the standard error output.

This is useful for any provider that returns an asynchronous operation handle (request token, operation ID, job ID, etc.) via `RETURNING *`.  When combined with [`return_vals`](manifest-file#resourcereturn_vals) in the manifest, the handle is captured as a `this.*` variable which the troubleshoot query can reference.

For example, given this manifest configuration:

```yaml
return_vals:
  create:
    - RequestToken
```

The troubleshoot query can reference the captured token:

```sql
/*+ troubleshoot:create */
SELECT OperationStatus, StatusMessage, ErrorCode
FROM awscc.cloud_control.resource_request
WHERE RequestToken = '{{ this.RequestToken }}'
AND region = '{{ region }}';
```

The operation qualifier (`:create`, `:update`, `:delete`) associates the troubleshoot query with a specific operation.  A plain `/*+ troubleshoot */` with no qualifier runs after **any** failed operation on the resource.

#### When troubleshoot runs

| Command | Trigger |
|---|---|
| `build` | Post-deploy statecheck fails (resource not in desired state after create or update) |
| `teardown` | Delete cannot be confirmed (resource still exists after delete) |

#### Execution behaviour

- **Fire-and-forget** - the query executes once (not polled like callbacks)
- **Tolerant rendering** - if template variables are missing the query is silently skipped
- **Non-blocking** - if the troubleshoot query itself fails, a warning is logged and the original error is still reported
- Results are logged as pretty-printed JSON at `info` level so they appear in default output alongside the failure message
- In dry-run mode, troubleshoot queries do not execute

#### Output format

When a troubleshoot query returns results, they are logged as pretty-printed JSON:

```
[vpc] troubleshoot diagnostics (create):

[
  {
    "OperationStatus": "FAILED",
    "StatusMessage": "The maximum number of VPCs has been reached.",
    "ErrorCode": "GeneralServiceException"
  }
]
```

#### Context

The troubleshoot query is rendered with the full template context at the point of failure, including:

- All global and resource properties from the manifest
- `callback.*` variables from `RETURNING *` data (if the preceding DML included `RETURNING *`)
- `this.*` fields captured by the `exists` query (if available)
- All variables exported by upstream resources

#### Troubleshoot options

| Option | Required | Default | Description |
|---|---|---|---|
| `retries` | no | 1 | Number of query attempts |
| `retry_delay` | no | 0 | Seconds to wait between attempts |

## Query options

Query options are used with query anchors to provide options for the execution of the query.

### `retries` and `retry_delay`

The `retries` and `retry_delay` query options are typically used for asynchronous or long running provider operations.  This will allow the resource time to become available or reach the desired state without failing the stack.

```sql
/*+ statecheck, retries=5, retry_delay=5 */
SELECT COUNT(*) as count FROM azure.resources.resource_groups
WHERE subscriptionId = '{{ subscription_id }}'
AND resourceGroupName = '{{ resource_group_name }}'
AND location = '{{ location }}'
AND JSON_EXTRACT(properties, '$.provisioningState') = 'Succeeded'
```

### `postdelete_retries` and `postdelete_retry_delay`

The `postdelete_retries` and `postdelete_retry_delay` query options are used in `exists` queries and are implemented specifically for `teardown` operations, allowing time for the resource to be deleted by the provider.

After each `delete` statement (and its `callback:delete`, if one is defined), the `exists` query is run immediately and, while the resource is still present, up to `postdelete_retries` more times with `postdelete_retry_delay` seconds between checks. The defaults are `postdelete_retries=10` and `postdelete_retry_delay=5`. If the resource is still present after the last check, the `delete` statement is re-issued according to the `delete` anchor's own `retries` and `retry_delay` options (default `retries=1`, a single attempt), after which the resource is reported as not confirmed deleted and the teardown moves on.

```sql
/*+ exists, postdelete_retries=10, postdelete_retry_delay=5 */
SELECT COUNT(*) as count FROM google.compute.instances
WHERE name = '{{ instance_name }}'
AND project = '{{ project }}'
AND zone = '{{ zone }}'
```

## Special Variables

In addition to the properties defined in the manifest, StackQL Deploy injects a set of built-in variables into every template context automatically.

| Variable | Scope | Description |
|---|---|---|
| `stack_name` | Global | Name of the stack as declared in the manifest |
| `stack_env` | Global | Environment name supplied to the CLI (`dev`, `prd`, etc.) |
| `resource_name` | Per-resource | Name of the resource currently being processed |
| `idempotency_token` | Per-resource | Stable UUID v4 for this resource for the lifetime of the session |
| `this.idempotency_token` | Per-resource (inside `.iql`) | Preferred alias — expands to `{{ <resource_name>.idempotency_token }}` |
| `<resource_name>.idempotency_token` | Global | Scoped form, usable in any downstream resource |
| `this.<field>` | Per-resource (inside `.iql`) | Fields captured from `exists` queries (see [identifier-based existence check](#identifier-based-existence-check)) |

### `idempotency_token`

`idempotency_token` is generated once per resource at session start and stays constant for all retries within that run.  Many providers (for example the AWS Cloud Control API) accept a client-side token to identify whether a request is a genuine new operation or a retry of an earlier one — `idempotency_token` is designed exactly for that purpose.

```sql
/*+ create */
INSERT INTO awscc.cloudformation.stacks(
  StackName,
  TemplateURL,
  ClientRequestToken,
  region
)
SELECT
  '{{ stack_name }}-{{ stack_env }}',
  '{{ template_url }}',
  '{{ this.idempotency_token }}',
  '{{ region }}'
RETURNING *
```

:::tip

Use `{{ this.idempotency_token }}` (which expands to `{{ <resource_name>.idempotency_token }}`) when writing queries inside a resource's own `.iql` file.  Use `{{ <resource_name>.idempotency_token }}` to access another resource's token from a downstream resource.

Unlike `{{ uuid() }}`, which generates a **new** UUID on every render, `idempotency_token` is stable for the entire session, making it safe to include in queries that may be retried.

:::

## Template Filters

StackQL Deploy uses a Jinja2-compatible templating engine and extends it with custom filters for infrastructure provisioning. For a complete reference of all available filters and special variables, see the [__Template Filters__](template-filters) documentation.

Here are a few commonly used filters:

- `from_json` - Converts JSON strings to native objects for iteration and manipulation
- `tojson` - Converts objects back to JSON strings
- `sql_escape` - Properly escapes SQL string literals for nested SQL statements
- `generate_patch_document` - Creates RFC6902-compliant patch documents for AWS resources
- `base64_encode` - Encodes strings as base64 for API fields requiring binary data

## Examples

### `resource` type example

This example is a `resource` file for a public IP address in a Google stack.

<File name='public_address.iql'>

```sql
/*+ exists */
SELECT COUNT(*) as count FROM google.compute.addresses
WHERE name = '{{ address_name }}'
AND project = '{{ project }}'
AND region = '{{ region }}'

/*+ create */
INSERT INTO google.compute.addresses
(
 project,
 region,
 name
) 
SELECT
'{{ project }}',
'{{ region }}',
'{{ address_name }}'

/*+ statecheck, retries=5, retry_delay=10 */
SELECT COUNT(*) as count FROM google.compute.addresses
WHERE name = '{{ address_name }}'
AND project = '{{ project }}'
AND region = '{{ region }}'

/*+ delete */
DELETE FROM google.compute.addresses
WHERE address = '{{ address_name }}' AND project = '{{ project }}'
AND region = '{{ region }}'

/*+ exports */
SELECT address
FROM google.compute.addresses
WHERE name = '{{ address_name }}'
AND project = '{{ project }}'
AND region = '{{ region }}'
```

</File>

### AWS Cloud Control API example with `callback`

This example shows a `resource` file for an S3 bucket using the AWS Cloud Control API (`awscc`), which uses an asynchronous mutation model.  The `RETURNING *` clause captures the `ProgressEvent` tracking handle and the `callback:create` / `callback:delete` blocks poll until the operation completes.

<File name='aws/s3/buckets.iql'>

```sql
/*+ exists */
SELECT COUNT(*) as count
FROM awscc.s3.buckets
WHERE region = '{{ region }}'
AND Identifier = '{{ bucket_name }}'

/*+ create */
INSERT INTO awscc.s3.buckets(BucketName, region)
SELECT '{{ bucket_name }}', '{{ region }}'
RETURNING *

/*+ callback:create, retries=10, retry_delay=15, short_circuit_field=ProgressEvent.OperationStatus, short_circuit_value=SUCCESS */
SELECT OperationStatus = 'SUCCESS' as success
FROM awscc.cloudcontrol.resource_request_statuses
WHERE region = '{{ region }}'
AND RequestToken = '{{ callback.ProgressEvent.RequestToken }}'

/*+ statecheck, retries=3, retry_delay=5 */
SELECT COUNT(*) as count
FROM awscc.s3.buckets
WHERE region = '{{ region }}'
AND Identifier = '{{ bucket_name }}'

/*+ exports */
SELECT '{{ bucket_name }}' as bucket_name

/*+ delete */
DELETE FROM awscc.s3.buckets
WHERE region = '{{ region }}'
AND Identifier = '{{ bucket_name }}'
RETURNING *

/*+ callback:delete, retries=10, retry_delay=15, short_circuit_field=ProgressEvent.OperationStatus, short_circuit_value=SUCCESS */
SELECT OperationStatus = 'SUCCESS' as success
FROM awscc.cloudcontrol.resource_request_statuses
WHERE region = '{{ region }}'
AND RequestToken = '{{ callback.ProgressEvent.RequestToken }}'
```

</File>

The corresponding manifest entry requires **no** `callback` section — callback behaviour is configured entirely in the `.iql` file:

```yaml
  - name: aws_s3_workspace_bucket
    file: aws/s3/buckets.iql
    props:
      - name: bucket_name
        value: "{{ stack_name }}-{{ stack_env }}-root-bucket"
      - name: region
        value: "ap-southeast-2"
    exports:
      - bucket_name
```

### Tag-based identifier discovery example (`awscc`)

This example demonstrates the **identifier capture** pattern for AWS Cloud Control (`awscc`) resources.  Resources are discovered using a CTE that cross-references `awscc.tagging.tagged_resources` with the provider's `*_list_only` resource, ensuring the resource actually exists (not just a stale tag record).  The captured field is then used via `{{ this.<field> }}` in `statecheck` and `exports` queries.

<File name='example_subnet.iql'>

```sql
/*+ exists */
WITH tagged_resources AS
(
    SELECT split_part(ResourceARN, '/', 2) as subnet_id
    FROM awscc.tagging.tagged_resources
    WHERE region = '{{ region }}'
    AND TagFilters = '{{ global_tags | to_aws_tag_filters }}'
    AND ResourceTypeFilters = '["ec2:subnet"]'
),
subnets AS
(
    SELECT subnet_id
    FROM awscc.ec2.subnets_list_only
    WHERE region = '{{ region }}'
)
SELECT r.subnet_id
FROM subnets r
INNER JOIN tagged_resources tr
ON r.subnet_id = tr.subnet_id;

/*+ statecheck, retries=5, retry_delay=5 */
SELECT COUNT(*) as count FROM
(
SELECT
AWS_POLICY_EQUAL(tags, '{{ subnet_tags }}') as test_tags
FROM awscc.ec2.subnets
WHERE Identifier = '{{ this.subnet_id }}'
AND region = '{{ region }}'
AND cidr_block = '{{ subnet_cidr_block }}'
AND vpc_id = '{{ vpc_id }}'
) t
WHERE test_tags = 1;

/*+ create */
INSERT INTO awscc.ec2.subnets (
 VpcId, CidrBlock, MapPublicIpOnLaunch, Tags, region
)
SELECT
 '{{ vpc_id }}', '{{ subnet_cidr_block }}', true,
 '{{ subnet_tags }}', '{{ region }}'
RETURNING *;

/*+ exports, retries=5, retry_delay=5 */
SELECT subnet_id, availability_zone
FROM awscc.ec2.subnets
WHERE Identifier = '{{ this.subnet_id }}'
AND region = '{{ region }}';

/*+ delete */
DELETE FROM awscc.ec2.subnets
WHERE Identifier = '{{ subnet_id }}'
AND region = '{{ region }}';
```

</File>

In this example:

1. **`exists`** — uses a CTE to cross-reference `awscc.tagging.tagged_resources` (filtered by stack tags via the [`to_aws_tag_filters`](template-filters#to_aws_tag_filters) filter) with `awscc.ec2.subnets_list_only`.  The `INNER JOIN` ensures the resource both has the expected tags **and** currently exists in the provider.  The returned `subnet_id` is captured as `{{ this.subnet_id }}`.
2. **`statecheck`** — uses `{{ this.subnet_id }}` to query `awscc.ec2.subnets` directly and verify properties including tags (via [`AWS_POLICY_EQUAL`](https://stackql.io/docs/language-spec/functions/json/aws_policy_equal)).
3. **`create`** — `INSERT` with `RETURNING *` to capture the Cloud Control API response.  Tags include `stackql:stack-name`, `stackql:stack-env`, and `stackql:resource-name` for future discovery.
4. **`exports`** — uses `{{ this.subnet_id }}` to query the resource and extract `subnet_id` and `availability_zone` for downstream resources.
5. **`delete`** — uses the exported `subnet_id` (from the `exports` query) with `Identifier`.

### `query` type example

This `query` example demonstrates retrieving the KMS key id for a given key alias in AWS.

<File name='get_logging_kms_key_id.iql'>

```sql
/*+ exports, retries=5, retry_delay=5 */
SELECT
target_key_id as logging_kms_key_id
FROM aws.kms.aliases
WHERE region = '{{ region }}'
AND Identifier = 'alias/{{ stack_name }}/{{ stack_env }}/logging';
```

</File>