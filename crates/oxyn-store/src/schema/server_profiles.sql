CREATE TABLE server_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    driver TEXT NOT NULL,
    params TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0)
) STRICT;
CREATE INDEX server_profiles_by_workspace ON server_profiles(workspace_id, name, id);

-- Removing a group removes membership only, never the child connection.
CREATE TABLE server_profile_members (
    connection_id TEXT PRIMARY KEY NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES server_profiles(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX server_profile_members_by_profile ON server_profile_members(profile_id);

-- The file itself refuses membership outside the profile's workspace/protocol.
CREATE TRIGGER server_profile_member_insert
BEFORE INSERT ON server_profile_members
WHEN NOT EXISTS (
    SELECT 1 FROM server_profiles p JOIN connections c
    ON c.workspace_id = p.workspace_id AND c.driver = p.driver
    WHERE p.id = NEW.profile_id AND c.id = NEW.connection_id
)
BEGIN
    SELECT RAISE(ABORT, 'invalid server profile membership');
END;
CREATE TRIGGER server_profile_member_update
BEFORE UPDATE ON server_profile_members
WHEN NOT EXISTS (
    SELECT 1 FROM server_profiles p JOIN connections c
    ON c.workspace_id = p.workspace_id AND c.driver = p.driver
    WHERE p.id = NEW.profile_id AND c.id = NEW.connection_id
)
BEGIN
    SELECT RAISE(ABORT, 'invalid server profile membership');
END;
CREATE TRIGGER server_profile_identity_update
BEFORE UPDATE OF workspace_id, driver ON server_profiles
WHEN NEW.workspace_id != OLD.workspace_id OR NEW.driver != OLD.driver
BEGIN
    SELECT RAISE(ABORT, 'server profile identity cannot change');
END;
CREATE TRIGGER server_profile_child_identity_update
BEFORE UPDATE OF workspace_id, driver ON connections
WHEN (NEW.workspace_id != OLD.workspace_id OR NEW.driver != OLD.driver)
AND EXISTS (SELECT 1 FROM server_profile_members WHERE connection_id = OLD.id)
BEGIN
    SELECT RAISE(ABORT, 'grouped connection identity cannot change');
END;
