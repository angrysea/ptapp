//! Thin single-table DynamoDB layer. Every item has string `pk` and `sk`;
//! the rest of the attributes are the serde_dynamo form of a domain struct.
//!
//! Key layout (see docs/ARCHITECTURE.md):
//!   USER#{sub}      PROFILE
//!   COACH#{coach}   CLIENT#{client} | INVITE#{code} | EXERCISE#{id} | PROGRAM#{id}
//!   INVITE#{code}   INVITE
//!   ADMIN           INVITE#{code} | COACH#{coach} | REQUEST#{sub}
//!   CLIENT#{client} INTAKE | WAIVER#{ts} | ASSIGN#{id} | SCHED#{date}#{id} | LOG#{date}#{id}
//!                   COMMENT#{ts}#{id} | METRIC#{date-id} | PHOTO#{date-id} | VIDEO#{date-id} | AUDIT#{ts}#{id}

use std::collections::HashMap;

use aws_sdk_dynamodb::types::{AttributeValue, PutRequest, WriteRequest};
use aws_sdk_dynamodb::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_dynamo::aws_sdk_dynamodb_1::{from_item, to_item};

use crate::error::{bad_request, ApiError, ApiResult};

pub type Item = HashMap<String, AttributeValue>;

#[derive(Clone)]
pub struct Store {
    pub ddb: Client,
    pub table: String,
}

pub fn s(v: impl Into<String>) -> AttributeValue {
    AttributeValue::S(v.into())
}

/// DynamoDB refuses items over 400 KB, which would surface as a 500. An
/// item's JSON is close to (and for text, a bit over) its DynamoDB size, so
/// this leaves headroom for the difference and the key attributes.
const MAX_ITEM_JSON_BYTES: usize = 350 * 1024;

/// Every write builds its item here, so this is where oversized ones are
/// turned into a 400 the user can act on.
pub fn keyed<T: Serialize>(pk: &str, sk: &str, value: &T) -> ApiResult<Item> {
    let json_len = serde_json::to_vec(value).map_err(|e| ApiError::Internal(format!("serialize: {e}")))?.len();
    if json_len > MAX_ITEM_JSON_BYTES {
        return Err(bad_request(format!(
            "this is too large to save ({} KB, limit {} KB); split it up or shorten the notes",
            json_len / 1024,
            MAX_ITEM_JSON_BYTES / 1024
        )));
    }
    let mut item: Item = to_item(value)?;
    item.insert("pk".into(), s(pk));
    item.insert("sk".into(), s(sk));
    Ok(item)
}

pub fn key(pk: &str, sk: &str) -> Item {
    HashMap::from([("pk".to_string(), s(pk)), ("sk".to_string(), s(sk))])
}

impl Store {
    pub async fn get<T: DeserializeOwned>(&self, pk: &str, sk: &str) -> ApiResult<Option<T>> {
        let out = self.ddb.get_item().table_name(&self.table).set_key(Some(key(pk, sk))).send().await?;
        out.item.map(from_item).transpose().map_err(Into::into)
    }

    pub async fn put<T: Serialize>(&self, pk: &str, sk: &str, value: &T) -> ApiResult<()> {
        self.ddb.put_item().table_name(&self.table).set_item(Some(keyed(pk, sk, value)?)).send().await?;
        Ok(())
    }

    /// Put only if no item with this key exists. Returns false if one did.
    pub async fn put_new<T: Serialize>(&self, pk: &str, sk: &str, value: &T) -> ApiResult<bool> {
        let res = self
            .ddb
            .put_item()
            .table_name(&self.table)
            .set_item(Some(keyed(pk, sk, value)?))
            .condition_expression("attribute_not_exists(pk)")
            .send()
            .await;
        match res {
            Ok(_) => Ok(true),
            Err(e) if e.as_service_error().is_some_and(|se| se.is_conditional_check_failed_exception()) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn delete(&self, pk: &str, sk: &str) -> ApiResult<()> {
        self.ddb.delete_item().table_name(&self.table).set_key(Some(key(pk, sk))).send().await?;
        Ok(())
    }

    /// All items under `pk` whose sort key starts with `prefix`.
    pub async fn query_prefix<T: DeserializeOwned>(&self, pk: &str, prefix: &str) -> ApiResult<Vec<T>> {
        self.query_raw(pk, "begins_with(sk, :p)", vec![(":p", s(prefix))], true).await
    }

    /// Items with `from <= sk <= to`, oldest first (or newest first if `ascending` is false).
    pub async fn query_between<T: DeserializeOwned>(&self, pk: &str, from: &str, to: &str, ascending: bool) -> ApiResult<Vec<T>> {
        self.query_raw(pk, "sk BETWEEN :a AND :b", vec![(":a", s(from)), (":b", s(to))], ascending).await
    }

    async fn query_raw<T: DeserializeOwned>(
        &self,
        pk: &str,
        sk_cond: &str,
        values: Vec<(&str, AttributeValue)>,
        ascending: bool,
    ) -> ApiResult<Vec<T>> {
        let mut out = Vec::new();
        let mut start: Option<Item> = None;
        loop {
            let mut q = self
                .ddb
                .query()
                .table_name(&self.table)
                .key_condition_expression(format!("pk = :pk AND {sk_cond}"))
                .expression_attribute_values(":pk", s(pk))
                .scan_index_forward(ascending)
                .set_exclusive_start_key(start.take());
            for (k, v) in &values {
                q = q.expression_attribute_values(*k, v.clone());
            }
            let page = q.send().await?;
            for item in page.items.unwrap_or_default() {
                out.push(from_item(item)?);
            }
            match page.last_evaluated_key {
                Some(k) if !k.is_empty() => start = Some(k),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Writes up to any number of items, 25 per request, retrying unprocessed ones.
    pub async fn batch_put(&self, items: Vec<Item>) -> ApiResult<()> {
        for chunk in items.chunks(25) {
            let mut reqs: Vec<WriteRequest> = chunk
                .iter()
                .map(|i| {
                    WriteRequest::builder()
                        .put_request(PutRequest::builder().set_item(Some(i.clone())).build().expect("item set"))
                        .build()
                })
                .collect();
            for attempt in 0..6 {
                let out = self.ddb.batch_write_item().request_items(&self.table, reqs).send().await?;
                reqs = out.unprocessed_items.and_then(|mut m| m.remove(&self.table)).unwrap_or_default();
                if reqs.is_empty() {
                    break;
                }
                if attempt == 5 {
                    return Err(ApiError::Internal("batch write left unprocessed items".into()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50 << attempt)).await;
            }
        }
        Ok(())
    }

    pub async fn batch_delete(&self, keys: Vec<(String, String)>) -> ApiResult<()> {
        for chunk in keys.chunks(25) {
            let mut reqs: Vec<WriteRequest> = chunk
                .iter()
                .map(|(pk, sk)| {
                    WriteRequest::builder()
                        .delete_request(
                            aws_sdk_dynamodb::types::DeleteRequest::builder()
                                .set_key(Some(key(pk, sk)))
                                .build()
                                .expect("key set"),
                        )
                        .build()
                })
                .collect();
            for attempt in 0..6 {
                let out = self.ddb.batch_write_item().request_items(&self.table, reqs).send().await?;
                reqs = out.unprocessed_items.and_then(|mut m| m.remove(&self.table)).unwrap_or_default();
                if reqs.is_empty() {
                    break;
                }
                if attempt == 5 {
                    return Err(ApiError::Internal("batch delete left unprocessed items".into()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50 << attempt)).await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::error::ApiError;

    #[test]
    fn oversized_items_are_a_bad_request() {
        let small = serde_json::json!({ "notes": "x".repeat(1000) });
        assert!(super::keyed("p", "s", &small).is_ok());
        let big = serde_json::json!({ "notes": "x".repeat(400 * 1024) });
        assert!(matches!(super::keyed("p", "s", &big), Err(ApiError::BadRequest(_))));
    }
}
