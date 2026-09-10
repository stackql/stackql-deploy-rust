---
title: teardown
hide_title: true
hide_table_of_contents: false
keywords:
  - stackql
  - stackql-deploy
  - infrastructure-as-code
  - configuration-as-data
tags:
  - stackql
  - stackql-deploy
  - infrastructure-as-code
  - configuration-as-data  
description: Documentation for the teardown command in StackQL Deploy
image: "/img/stackql-cover.png"
---

# <span className="docFieldHeading">`teardown`</span>

Command used to deprovision and remove resources in a specified stack in a given environment.

* * *

## Syntax

<code>stackql-deploy <span className="docFieldHeading">teardown</span> STACK_DIR STACK_ENV [FLAGS]</code>

* * *

## Arguments

| Argument | Description | Example |
|--|--|--|
| `STACK_DIR` | The directory containing the stack configuration files | `my-stack` |
| `STACK_ENV` | The target environment for tearing down the stack | `dev` |

:::info

`STACK_DIR` can be an absolute or relative path.  

`STACK_ENV` is a user-defined environment symbol (e.g., `dev`, `sit`, `prd`) used to tear down your stack in different environments.

:::

## Optional Flags

| Flag | Description | Example |
|--|--|--|
| <span class="nowrap">`--log-level`</span> | Set the logging level. Default is `INFO` | `--log-level DEBUG` |
| <span class="nowrap">`--env-file`</span> | Specify an environment variables file. Default is `.env` | `--env-file .env` |
| <span class="nowrap">`-e`</span> <span class="nowrap">`--env`</span> | Set additional environment variables (can be used multiple times) | `--env DB_USER=admin` |
| <span class="nowrap">`--dry-run`</span> | Perform a dry run of the operation. No changes will be made | |
| <span class="nowrap">`--show-queries`</span> | Display the queries executed in the output logs | |
| <span class="nowrap">`--on-failure`</span> | What to do when a `delete` statement is rejected by the provider: `error` (default) aborts the run at the first failure, `ignore` logs the failure, reports the resource as not confirmed deleted, and continues with the next resource | `--on-failure ignore` |

:::tip

Exported variables specified as `protected` in the respective resource definition in the `stackql_manifest.yml` file are obfuscated in the logs by default.

:::

:::info

With `--on-failure ignore`, fatal errors (network, authentication, and stackql planner errors) still abort the run. A delete that is dispatched but cannot be confirmed within its retry budget never aborts the run on its own; resources whose delete could not be confirmed are listed in a summary at the end of the teardown, and the exit status is still `0`.

Resources that are skipped during teardown - because they carry `skip_on_delete: true`, or because a query they depend on could not be rendered with real values - are logged and left in place.

:::

* * *

## Examples

### Teardown a stack in a target environment

Teardown the stack defined in the `azure-stack` directory in the `sit` environment, setting additional environment variables:

```bash
stackql-deploy teardown azure-stack sit \
-e AZURE_SUBSCRIPTION_ID=631d1c6d-0000-0000-0000-688bfe4e1468
```
