//! 导航菜单 Repository 的 PostgreSQL 实现。
//! 两级限制：create/update 时校验父节点存在且父节点自身无 parent；
//! 排序为事务内原子批量更新。

use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::navigation::{
    NavigationError, NavigationItem, NavigationItemUpdate, NavigationRepository, NewNavigationItem,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresNavigationRepository {
    pool: PgPool,
}

impl PostgresNavigationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct NavigationItemRow {
    id: i64,
    parent_id: Option<i64>,
    label: String,
    target_type: String,
    target_id: Option<i64>,
    url: Option<String>,
    open_in_new_tab: bool,
    visible: bool,
    sort_order: i32,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<NavigationItemRow> for NavigationItem {
    type Error = NavigationError;

    fn try_from(row: NavigationItemRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            parent_id: row.parent_id,
            label: row.label,
            target_type: row
                .target_type
                .parse()
                .map_err(|_| NavigationError::InvalidTargetType)?,
            target_id: row.target_id,
            url: row.url,
            open_in_new_tab: row.open_in_new_tab,
            visible: row.visible,
            sort_order: row.sort_order,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const NAVIGATION_COLUMNS: &str = "id, parent_id, label, target_type, target_id, url, \
    open_in_new_tab, visible, sort_order, created_at, updated_at";

fn map_sqlx(error: sqlx::Error) -> NavigationError {
    if let sqlx::Error::Database(db) = &error {
        match db.code().as_deref() {
            // 23503 = foreign_key_violation：父节点不存在。
            Some("23503") => return NavigationError::InvalidHierarchy,
            // 23514 = check_violation：target 字段与 target_type 不匹配。
            Some("23514") => return NavigationError::InvalidTarget,
            _ => {}
        }
    }
    tracing::error!(error = %error, "navigation repository operation failed");
    NavigationError::StoreUnavailable
}

/// 校验目标字段与层级限制（父节点必须存在且自身不能再有 parent，保证最多两级）。
async fn validate_hierarchy(pool: &PgPool, parent_id: Option<i64>) -> Result<(), NavigationError> {
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    let parent =
        logged_query_as::<(Option<i64>,)>("SELECT parent_id FROM navigation_items WHERE id = $1")
            .bind(parent_id)
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(NavigationError::InvalidHierarchy)?;
    if parent.0.is_some() {
        return Err(NavigationError::InvalidHierarchy);
    }
    Ok(())
}

#[async_trait]
impl NavigationRepository for PostgresNavigationRepository {
    async fn create(&self, item: NewNavigationItem) -> Result<NavigationItem, NavigationError> {
        aries_core::navigation::validate_label(&item.label)?;
        aries_core::navigation::validate_target(
            item.target_type,
            item.target_id,
            item.url.as_deref(),
        )?;
        validate_hierarchy(&self.pool, item.parent_id).await?;

        let query = format!(
            "INSERT INTO navigation_items \
             (parent_id, label, target_type, target_id, url, open_in_new_tab, visible, sort_order) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             RETURNING {NAVIGATION_COLUMNS}"
        );
        let row = logged_query_as::<NavigationItemRow>(&query)
            .bind(item.parent_id)
            .bind(item.label.trim())
            .bind(item.target_type.as_str())
            .bind(item.target_id)
            .bind(&item.url)
            .bind(item.open_in_new_tab)
            .bind(item.visible)
            .bind(item.sort_order)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn find(&self, item_id: i64) -> Result<Option<NavigationItem>, NavigationError> {
        let query = format!("SELECT {NAVIGATION_COLUMNS} FROM navigation_items WHERE id = $1");
        let row = logged_query_as::<NavigationItemRow>(&query)
            .bind(item_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn list(&self) -> Result<Vec<NavigationItem>, NavigationError> {
        let query = format!(
            "SELECT {NAVIGATION_COLUMNS} FROM navigation_items \
             ORDER BY parent_id NULLS FIRST, sort_order, id"
        );
        let rows = logged_query_as::<NavigationItemRow>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()
    }

    async fn list_visible(&self) -> Result<Vec<NavigationItem>, NavigationError> {
        let query = format!(
            "SELECT {NAVIGATION_COLUMNS} FROM navigation_items \
             WHERE visible = true \
             ORDER BY parent_id NULLS FIRST, sort_order, id"
        );
        let rows = logged_query_as::<NavigationItemRow>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()
    }

    async fn update(
        &self,
        item_id: i64,
        update: NavigationItemUpdate,
    ) -> Result<NavigationItem, NavigationError> {
        aries_core::navigation::validate_label(&update.label)?;
        aries_core::navigation::validate_target(
            update.target_type,
            update.target_id,
            update.url.as_deref(),
        )?;
        // 禁止把节点挂到自身或形成第三级。
        if update.parent_id == Some(item_id) {
            return Err(NavigationError::InvalidHierarchy);
        }
        validate_hierarchy(&self.pool, update.parent_id).await?;
        // 有子节点的节点不允许再挂到别人下面（会变成第三级）。
        if update.parent_id.is_some() {
            let has_children = logged_query_scalar::<bool>(
                "SELECT EXISTS(SELECT 1 FROM navigation_items WHERE parent_id = $1)",
            )
            .bind(item_id)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
            if has_children {
                return Err(NavigationError::InvalidHierarchy);
            }
        }

        let query = format!(
            "UPDATE navigation_items SET parent_id = $2, label = $3, target_type = $4, \
             target_id = $5, url = $6, open_in_new_tab = $7, visible = $8, sort_order = $9, \
             updated_at = now() \
             WHERE id = $1 \
             RETURNING {NAVIGATION_COLUMNS}"
        );
        let row = logged_query_as::<NavigationItemRow>(&query)
            .bind(item_id)
            .bind(update.parent_id)
            .bind(update.label.trim())
            .bind(update.target_type.as_str())
            .bind(update.target_id)
            .bind(&update.url)
            .bind(update.open_in_new_tab)
            .bind(update.visible)
            .bind(update.sort_order)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(NavigationError::NotFound)?;
        row.try_into()
    }

    async fn delete(&self, item_id: i64) -> Result<(), NavigationError> {
        // 先检查子节点，给出可读的 Conflict 而非原始 FK 错误。
        let has_children = logged_query_scalar::<bool>(
            "SELECT EXISTS(SELECT 1 FROM navigation_items WHERE parent_id = $1)",
        )
        .bind(item_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if has_children {
            return Err(NavigationError::Conflict);
        }
        let result = logged_query("DELETE FROM navigation_items WHERE id = $1")
            .bind(item_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(NavigationError::NotFound);
        }
        Ok(())
    }

    async fn reorder(&self, ordered_item_ids: Vec<i64>) -> Result<(), NavigationError> {
        // 事务内按入参顺序重写 sort_order，客户端一次性提交全量顺序。
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        if ordered_item_ids.is_empty() {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(());
        }
        let sort_orders: Vec<i32> = (0..ordered_item_ids.len())
            .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
            .collect();
        let result = logged_query(
            "UPDATE navigation_items SET sort_order = data.sort_order \
             FROM (SELECT unnest($1::bigint[]) AS id, unnest($2::int[]) AS sort_order) AS data \
             WHERE navigation_items.id = data.id",
        )
        .bind(&ordered_item_ids)
        .bind(&sort_orders)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != ordered_item_ids.len() as u64 {
            return Err(NavigationError::NotFound);
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(())
    }
}
