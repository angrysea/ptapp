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

## After the first deploy

* **MFA for coaches.** The pool allows TOTP (`MfaConfiguration: OPTIONAL`). Coaches enable it from the hosted UI. Enforcing it for the coach role is a Phase 1b item.
* **Emails.** Cognito's default sender is limited to about 50 emails/day. Before real users, configure SES (`EmailConfiguration` on the user pool) with a verified domain.
* **Custom domain.** Add an ACM certificate (us-east-1) and alias to CloudFront and to the Cognito domain, then add the new URL to the client's callback and logout URLs and the CORS origins.
* **Production stage.** Use `STAGE=prod`, a different domain prefix and a separate AWS account if you can. The table and media bucket have `DeletionPolicy: Retain`.
* **Backups.** PITR gives 35 days of restore points; also schedule AWS Backup for the table and **test a restore** (BRD reliability NFR).
* **Legal.** Have the waiver text and PAR-Q wording (`web/js/views/intake.js`) reviewed, and add a privacy policy covering CCPA and app-store health-data rules before launch.

## Rollback

`sam deploy` is a CloudFormation change set. Redeploy the previous git commit to roll back code. Data tables are retained if the stack is deleted.
