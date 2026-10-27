# Deploying to AWS

Prerequisites (WSL / Linux / macOS / CI): Rust with the `aarch64-unknown-linux-gnu` target, `cargo-lambda`, `aws-sam-cli` and the AWS CLI v2, all configured with credentials for the target account (`aws configure` or `AWS_PROFILE`). See DEVELOPMENT.md for install commands.

```bash
export AWS_REGION=us-east-1
export COGNITO_DOMAIN_PREFIX=yourbrand-pt-dev   # must be globally unique
STAGE=dev ./scripts/deploy.sh
```

The script:

1. `cargo lambda build --release --arm64 --bin api` → `target/lambda/api/bootstrap`
2. `sam deploy` creates the stack `ptapp-dev`: DynamoDB (PITR, SSE, TTL), media bucket (private, AES256, TLS-only, CORS for the app origin), Cognito user pool, hosted domain and web client (code + PKCE), HTTP API with JWT authorizer, throttling and CORS, Lambda (arm64, 256 MB, 30-day logs), and web bucket with CloudFront (OAC, security headers)
3. copies `web/` to a temporary folder and writes its `js/config.js` from the stack outputs (your checkout is left untouched)
4. stamps the service-worker version in the copy, syncs it to S3 (no-cache on `index.html`, `sw.js`, `config.js`) and invalidates CloudFront

It prints the app URL (`https://dxxxx.cloudfront.net`). The first coach signs up through **Sign in → Sign up** on the Cognito page, then picks "I'm a coach".

## Deploying from GitHub Actions

`.github/workflows/ci.yml` runs on every push to `main` and on every pull request: clippy (warnings fail the build), unit tests, `cfn-lint`, a syntax check of the web app, the API journey against moto, and the Playwright browser journey (screenshots are attached to failed runs). It deploys **only** when started by hand: **Actions → CI → Run workflow**, tick *Deploy*, pick the stage. It authenticates to AWS with OIDC, so no AWS keys are stored in GitHub.

One-time setup per AWS account and stage:

1. Create the deploy roles (this also creates the account's GitHub OIDC provider; pass `CreateOidcProvider=false` if one already exists):
   ```bash
   aws cloudformation deploy --template-file scripts/ci-deploy-role.yaml \
     --stack-name ptapp-github-deploy-dev --capabilities CAPABILITY_NAMED_IAM \
     --parameter-overrides Environment=dev
   aws cloudformation describe-stacks --stack-name ptapp-github-deploy-dev \
     --query "Stacks[0].Outputs[?OutputKey=='RoleArn'].OutputValue" --output text
   ```
   This creates three layers, so a compromised CI job cannot take over the account:
   * the **CI role**, which can only run CloudFormation on the `ptapp-<stage>` stack, upload to its own artifact bucket, publish the web app and invalidate CloudFront. It has no direct access to IAM or the app's data;
   * a **CloudFormation execution role** (`cfn-exec-ptapp-<stage>`) that creates the stack's resources. It can only create IAM roles that carry the boundary below;
   * a **permissions boundary** (`ptapp-<stage>-boundary`) that caps every role in the stack at the app's own table, buckets and logs. When the app needs a new AWS service (SQS for video, for example), add it to the boundary in `scripts/ci-deploy-role.yaml`.

   CI can submit any template, so the execution role is the real limit of what a CI deploy can do. It only covers the services the stack uses: DynamoDB, S3, Lambda and logs are limited to this stage's names, while Cognito, API Gateway and CloudFront can't be limited by name because AWS generates their IDs. For production, use a separate AWS account per stage so those three can't reach anything else. If a first CI deploy fails with `AccessDenied`, the error names the missing action; add it to `stack-services`.

   If you deployed this stage from your own machine before, its function role has no boundary yet. The first CI deploy attaches it. Until then, don't attach extra policies to `ptapp-<stage>-*` roles by hand.
2. In GitHub, **Settings → Environments → New environment** named `dev`:
   * Under **Deployment branches and tags**, choose **Selected branches** and add `main`. **This is required:** AWS trusts any job that runs in this environment, so without the rule a workflow edited on another branch could deploy.
   * Optionally add required reviewers for an approval step.
   * Add these variables:
     * `AWS_DEPLOY_ROLE_ARN`: the `RoleArn` output from step 1
     * `COGNITO_DOMAIN_PREFIX`: as above, globally unique
     * `AWS_REGION`: optional, defaults to `us-east-1`
3. Repeat with `Environment=prod` and a `prod` environment for production.

CI deploys set `DEPLOY_ROLE_STACK=1`, which makes `deploy.sh` use the execution role, the boundary and the artifact bucket. Deploying from your own machine without it works as before.

To deploy on every push to `main` instead, add `push` to the deploy job's `if:` condition in `ci.yml`.

## After the first deploy

* **MFA for coaches.** The pool allows TOTP (`MfaConfiguration: OPTIONAL`). Coaches enable it from the hosted UI. Enforcing it for the coach role is a Phase 1b item.
* **Emails.** Cognito's default sender is limited to about 50 emails/day. Before real users, configure SES (`EmailConfiguration` on the user pool) with a verified domain.
* **Custom domain.** Add an ACM certificate (us-east-1) and alias to CloudFront and to the Cognito domain, then add the new URL to the client's callback and logout URLs and the CORS origins.
* **Production stage.** Use `STAGE=prod`, a different domain prefix and a separate AWS account if you can. The table and media bucket have `DeletionPolicy: Retain`.
* **Backups.** PITR gives 35 days of restore points; also schedule AWS Backup for the table and **test a restore** (BRD reliability NFR).
* **Legal.** Have the waiver text and PAR-Q wording (`web/js/views/intake.js`) reviewed, and add a privacy policy covering CCPA and app-store health-data rules before launch.

## Rollback

`sam deploy` is a CloudFormation change set. Redeploy the previous git commit to roll back code. Data tables are retained if the stack is deleted.
