# 0051: Clear internal container staging only after verified import

Status: approved by the user on 2026-10-22.

The user selected automatic removal of the application's internal encrypted
container copy after a successful, fully verified new-book import. Their
original downloaded or external-media package must remain untouched. Failed
or interrupted internal staging remains available for inspection and recovery.

The owned importing worker records directory and object identities while it
creates the stage. Cleanup is permitted only after the complete recipient
importer has authenticated, validated and published the new book. It checks the
exact known entry set and recorded identities before unlinking known objects;
it does not recursively remove a path merely because its name looks familiar.
Unknown entries, replaced objects/directories and symlinks prevent cleanup.

An optional cleanup failure does not turn an already published, verified book
into a failed import. The outcome carries a cleanup warning, exposed by HTTP as
the existing boolean `cleanup_warning` without local paths or OS error details.
Book selection remains unchanged. Startup does not adopt old staging folders
as cleanup authority; interrupted-work review remains conservative.

Regression coverage includes successful removal, preserved original package,
failed import retention, unknown entries, substituted objects/directories and
symlinks. These checks narrow path replacement windows; they do not claim to
eliminate actively racing same-account filesystem syscalls.
