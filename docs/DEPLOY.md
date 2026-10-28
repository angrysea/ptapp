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

It prints the app URL (`https://dxxxx.cloudfront.net`).

**First admin.** Accounts are by invitation only. Pass `ADMIN_EMAILS` (comma-separated) on every deploy. Someone whose email is on the list signs up through **Sign in → Sign up** on the Cognito page and sees **Set up admin account**. Their admin account then stays even if the list changes.

**Invitation emails (SES).** Pass `INVITE_FROM` (e.g. `invites@gymtxcoach.com`) on every deploy. The stack creates an SES identity for its domain, signed with DKIM:

1. Add the three CNAME records from the stack outputs `DkimRecord1`–`DkimRecord3` to the domain's DNS (gymtxcoach.com is hosted on Cloudflare: add each as a CNAME with the proxy turned off (DNS only)). SES verifies the domain within about an hour of the records appearing.
2. New SES accounts are in the **sandbox**: they can only email addresses verified in SES. Request **production access** (SES console › Account dashboard › Request production access; AWS usually answers within a day).

Until both are done, invitations are still created and the admin page shows the link to share by hand.

If the sender domain is already an SES identity in this account and region (for example, created in the SES console), deploy with `CREATE_SENDER_IDENTITY=false`: the stack then uses that identity, and its three DKIM records are on the identity's page in the SES console (or `aws sesv2 get-email-identity --email-identity <domain>`) instead of in the stack outputs.

## Signing in and deploying from Windows

Deploys run from a developer machine, not from GitHub. (The AWS Organization this app deploys into blocks creating the OIDC provider GitHub would need.) GitHub CI (`.github/workflows/ci.yml`) still runs clippy, the unit tests, `cfn-lint`, a web syntax check, the API journey against moto and the Playwright journey on every push and pull request.

1. Install the tools once: `winget install Amazon.AWSCLI Amazon.SAM-CLI zig.zig`, `pip install cargo-lambda` and `rustup target add aarch64-unknown-linux-gnu`.
2. Sign in with your normal AWS console login: `aws login --profile ptapp` (it opens the browser; the CLI refreshes the temporary credentials itself). Use `aws configure sso` only if your account uses an IAM Identity Center portal.
3. Deploy from Git Bash in a new terminal (so it sees the new tools). SAM installs as `sam.cmd`, which Git Bash cannot call as `sam`, so wrap it:
   ```bash
   sam() { "/c/Program Files/Amazon/AWSSAMCLI/bin/sam.cmd" "$@"; }; export -f sam
   AWS_PROFILE=ptapp AWS_REGION=us-east-2 STAGE=dev COGNITO_DOMAIN_PREFIX=ptapp-online ADMIN_EMAILS=you@example.com ./scripts/deploy.sh
   ```

## After the first deploy

* **MFA for coaches.** The pool allows TOTP (`MfaConfiguration: OPTIONAL`). Coaches enable it from the hosted UI. Enforcing it for the coach role is a Phase 1b item.
* **Emails.** Cognito's default sender is limited to about 50 emails/day. Before real users, configure SES (`EmailConfiguration` on the user pool) with a verified domain.
* **Custom domain** (e.g. `gymtxcoach.com`, with `www` redirecting to it):
  1. Request a certificate in **us-east-1** (CloudFront only uses certificates there):
     `aws acm request-certificate --region us-east-1 --domain-name gymtxcoach.com --subject-alternative-names www.gymtxcoach.com --validation-method DNS`
  2. Add the two validation CNAMEs from `aws acm describe-certificate --region us-east-1 --certificate-arn <arn>` to the domain's DNS and wait for the status `ISSUED`.
  3. Deploy with `APP_DOMAIN=gymtxcoach.com APP_CERT_ARN=<arn>`. CloudFront then answers for the domain and `www` (redirected), and the Cognito callback and logout URLs, API and media CORS, and invitation links follow it. The CloudFront address keeps working.
  4. In DNS, point `gymtxcoach.com` and `www` at the `CloudFrontDomain` output with CNAMEs, **DNS only** (on Cloudflare, the grey cloud; Cloudflare flattens the bare-domain CNAME).

  The Cognito sign-in page keeps its `*.amazoncognito.com` address; giving it its own (e.g. `auth.gymtxcoach.com`) needs another us-east-1 certificate.
* **Production stage.** Use `STAGE=prod`, a different domain prefix and a separate AWS account if you can. The table and media bucket have `DeletionPolicy: Retain`.
* **Backups.** PITR gives 35 days of restore points (BRD reliability NFR). Restoring makes a *new* table, which the stack then has to point at (`TABLE_NAME`); the media bucket isn't covered (S3 versioning would be).
  * **Restore test (2026-10-28, dev):**
    * **Restore:** `aws dynamodb restore-table-to-point-in-time --source-table-name ptapp-dev --target-table-name ptapp-dev-restore-test --use-latest-restorable-time --billing-mode-override PAY_PER_REQUEST` took **183 s** to become ACTIVE.
    * **Contents:** the restore had 150 items, against 162 in the live table when counted. The latest restorable point is a few minutes old, so the 12 were writes made since. A known program came back intact.
    * **Cleanup:** the test table was then deleted. Repeat this after major data-model changes.
* **Alarms.** `ALARM_EMAIL` (deploy.sh) gets an email when the API Lambda errors 5+ times or answers 10+ requests with 5xx in 5 minutes, or is throttled; confirm the subscription email AWS sends first. The alarms are `ptapp-{stage}-api-*` in CloudWatch.
* **Legal.** Have the waiver text and PAR-Q wording (`web/js/views/intake.js`) reviewed, and add a privacy policy covering CCPA and app-store health-data rules before launch.

## Rollback

`sam deploy` is a CloudFormation change set. Redeploy the previous git commit to roll back code. Data tables are retained if the stack is deleted.
