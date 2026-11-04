-- An example Kaye plugin. Copy it into the plugins folder (Plugins > Open Plugins
-- Folder), then pick Plugins > Reload Plugins. Each kaye.command adds a menu entry.
--
-- kaye.command(name, function)   adds a command to the Plugins menu
-- kaye.selected()                the selected bones
-- kaye.bones()                   every joint in the rig being worked on
-- kaye.findBone(name)            a bone by name, or nil
-- kaye.boneName(bone)            a bone's name
-- kaye.getPose(bone)             { x, y, z, rx, ry, rz }, with rotation in degrees
-- kaye.setPose(bone, pose)       anything left out of the table stays as it is
-- kaye.setKey(bone)              keys a bone on the current frame
-- kaye.getFrame()                the current frame
-- kaye.setFrame(frame)           moves to a frame
-- kaye.getRange()                the first and last frame
-- kaye.mirror(bones)             mirrors bones across the rig's middle
-- kaye.message(text)             shows a message

kaye.command("Key Every Bone", function()
	local bones = kaye.bones()
	for _, bone in bones do
		kaye.setKey(bone)
	end
	kaye.message(`Keyed {#bones} bones`)
end)

kaye.command("Exaggerate Pose", function()
	for _, bone in kaye.selected() do
		local pose = kaye.getPose(bone)
		kaye.setPose(bone, { rx = pose.rx * 1.25, ry = pose.ry * 1.25, rz = pose.rz * 1.25 })
		kaye.setKey(bone)
	end
end)
