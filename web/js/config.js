// Local development config. scripts/deploy.sh publishes a generated copy with
// the deployed stack's values and leaves this file as is. With devUser set, the app skips Cognito and sends
// an x-dev-user header (only honoured by an API running with DEV_AUTH=1).
export default {
  apiUrl: "http://localhost:9000",
  cognitoDomain: "",
  clientId: "",
  devUser: true,
};
