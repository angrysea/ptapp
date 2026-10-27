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

1. Create the deploy role (it also creates the account's GitHub OIDC provider; pass `CreateOidcProvider=false` if one already exists):
   ```bash
   aws cloudformation deploy --template-file scripts/ci-deploy-role.yaml \
     --stack-name ptapp-github-deploy-dev --capabilities CAPABILITY_NAMED_IAM \
     --parameter-overrides Environment=dev
   aws cloudformation describe-stacks --stack-name ptapp-github-deploy-dev \
     --query "Stacks[0].Outputs[?OutputKey=='RoleArn'].OutputValue" --output text
   ```
   The role trusts only workflow jobs running in the GitHub environment of the same name. It has `PowerUserAccess` plus IAM rights limited to the stack's own `ptapp-<stage>-*` roles.
2. In GitHub, **Settings → Environments → New environment** named `dev` (add required reviewers if you want an approval step), with these variables:
   * `AWS_DEPLOY_ROLE_ARN`: the role ARN from step 1
   * `COGNITO_DOMAIN_PREFIX`: as above, globally unique
   * `AWS_REGION`: optional, defaults to `us-east-1`
3. Repeat with `Environment=prod` and a `prod` environment for production.

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
