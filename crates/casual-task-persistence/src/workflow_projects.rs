/// Every project on this workflow, as `(project_id, team_ids, actor_is_member)`.
///
/// The authority question for a workflow edit is asked once per project in this
/// list. Membership comes back in the same row to avoid per-project resolution.
pub async fn projects_on(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    actor: Uuid,
) -> Result<Vec<(Uuid, Vec<Uuid>, bool)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id,
                ARRAY(SELECT pt.team_id FROM project_team pt
                       WHERE pt.project_id = p.id ORDER BY pt.team_id),
                EXISTS (SELECT 1 FROM project_membership pm
                         WHERE pm.project_id = p.id AND pm.user_id = $2)
           FROM project p
          WHERE p.workflow_id = $1 AND p.deleted_at IS NULL
       ORDER BY p.id",
    )
    .bind(workflow)
    .bind(actor)
    .fetch_all(scoped.conn())
    .await
}
