package server

import (
	"fmt"
	"math"
	"time"

	"go.uber.org/zap"
)

const maxActionTransitCompensation = 30 * time.Second

// duplicateChangeWindow is how long a repeat change_track to the playing song is ignored.
const duplicateChangeWindow = 5 * time.Second

func roomClientsLocked(room *Room) []*Client {
	clients := make([]*Client, 0, len(room.Clients))
	for _, client := range room.Clients {
		if client != nil {
			clients = append(clients, client)
		}
	}
	return clients
}

func (s *Server) sendPolicyQueueSnapshot(client *Client, state *RoomState) {
	if client == nil || state == nil || client.uaTier == uaAllow || client.uaTier == uaBlock {
		return
	}
	trackID := ""
	if state.CurrentTrack != nil {
		trackID = state.CurrentTrack.ID
	}
	client.sendMessage(s.logger, MsgTypeSyncPlayback, PlaybackActionPayload{
		Action:     ActionSyncQueue,
		TrackID:    trackID,
		Queue:      append([]TrackInfo(nil), state.Queue...),
		Position:   state.Position,
		ServerTime: state.LastUpdate,
		Revision:   state.Revision,
	})
}

func validatePlaybackTrack(c *Client, logger *zap.Logger, state *RoomState, p *PlaybackActionPayload) bool {
	if state.CurrentTrack == nil {
		return true
	}
	if p.TrackID == "" {
		p.TrackID = state.CurrentTrack.ID
		return true
	}
	if p.TrackID != state.CurrentTrack.ID {
		c.sendError(logger, "stale_track", "Playback action targets a stale track")
		return false
	}
	return true
}

// clampPlaybackPosition bounds a position without trusting reported track
// durations, which clients fill with placeholders when unknown.
func clampPlaybackPosition(position int64) int64 {
	return min(max(position, 0), MaxTrackDuration)
}

// compensateActionTransit advances a playing position by the time the action
// spent in transit. Without a capture timestamp it falls back to half the
// host's measured round trip.
func compensateActionTransit(position, capturedAtServerTime, nowMs, rttMs int64) int64 {
	elapsed := rttMs / 2
	if capturedAtServerTime > 0 {
		elapsed = nowMs - capturedAtServerTime
	}
	if elapsed <= 0 || elapsed > maxActionTransitCompensation.Milliseconds() {
		// Future or ancient capture times come from a miscalibrated clock.
		return position
	}
	if position > math.MaxInt64-elapsed {
		return math.MaxInt64
	}
	return position + elapsed
}

func sanitizeQueue(queue []TrackInfo) []TrackInfo {
	sanitized := make([]TrackInfo, 0, len(queue))
	for _, track := range queue {
		if !sanitizeTrackInfo(&track) {
			continue
		}
		sanitized = append(sanitized, track)
		if len(sanitized) == MaxQueueSize {
			break
		}
	}
	return sanitized
}

// queueWireSize estimates the encoded size of a queue snapshot.
func queueWireSize(queue []TrackInfo) int {
	size := 0
	for i := range queue {
		t := &queue[i]
		size += len(t.ID) + len(t.Title) + len(t.Artist) + len(t.Album) + len(t.Thumbnail) + len(t.SuggestedBy) + 32
	}
	return size
}

func containsTrackID(queue []TrackInfo, trackID string) bool {
	for i := range queue {
		if queue[i].ID == trackID {
			return true
		}
	}
	return false
}

// advanceQueue consumes the upcoming queue up to and including trackID, which
// is what playing that entry means. A track that is not queued leaves it as is.
func advanceQueue(queue []TrackInfo, trackID string) []TrackInfo {
	for i := range queue {
		if queue[i].ID == trackID {
			return append([]TrackInfo(nil), queue[i+1:]...)
		}
	}
	return queue
}

// syncSnapshotLocked builds a sync_state for the room. Caller holds room.mu.
func syncSnapshotLocked(room *Room, nowMs int64) *SyncStatePayload {
	return &SyncStatePayload{
		CurrentTrack: cloneTrackInfo(room.State.CurrentTrack),
		IsPlaying:    room.State.IsPlaying,
		Position:     livePlaybackPosition(room.State, nowMs),
		LastUpdate:   nowMs,
		Queue:        append([]TrackInfo(nil), room.State.Queue...),
		Volume:       room.State.Volume,
		Revision:     room.State.Revision,
		QueueTitle:   room.State.QueueTitle,
	}
}

func (s *Server) handlePlaybackAction(c *Client, payload []byte) {
	room := c.currentRoom()
	if room == nil {
		c.sendError(s.logger, "not_in_room", "You are not in a room")
		return
	}
	room.mu.RLock()
	isMember := room.Clients[c.clientID()] == c
	room.mu.RUnlock()
	if !isMember {
		c.sendError(s.logger, "not_in_room", "You are not an active room member")
		return
	}

	var p PlaybackActionPayload
	if err := decodePayload(payload, MsgTypePlaybackAction, &p); err != nil {
		c.sendError(s.logger, "invalid_payload", "Invalid playback action payload")
		return
	}
	if p.Action == "" {
		c.sendError(s.logger, "missing_action", "Action is required")
		return
	}
	p.QueueTitle = sanitizeString(p.QueueTitle, MaxTrackTitleLength)

	room.syncMu.Lock()
	defer room.syncMu.Unlock()
	room.mu.Lock()

	if room.Clients[c.clientID()] != c {
		room.mu.Unlock()
		c.sendError(s.logger, "not_in_room", "You are not an active room member")
		return
	}

	nowMs := serverNowMs()
	if p.HostSequence > 0 {
		// Observations sampled before a newer action must not overwrite it.
		if p.HostSequence <= c.lastSequence {
			room.mu.Unlock()
			c.sendError(s.logger, "stale_action", "Playback action is older than one already applied")
			return
		}
		c.lastSequence = p.HostSequence
	}
	rttMs := c.roundTripMs()
	switch p.Action {
	case ActionPlay:
		if room.State.CurrentTrack == nil {
			room.mu.Unlock()
			c.sendError(s.logger, "no_track", "Cannot play without a track")
			return
		}
		if p.Position < 0 {
			room.mu.Unlock()
			c.sendError(s.logger, "invalid_position", "Position cannot be negative")
			return
		}
		if !validatePlaybackTrack(c, s.logger, room.State, &p) {
			room.mu.Unlock()
			return
		}
		p.Position = clampPlaybackPosition(compensateActionTransit(p.Position, p.CapturedAtServerTime, nowMs, rttMs))
		room.State.IsPlaying = true
		room.State.Position = p.Position
		room.State.LastUpdate = nowMs

	case ActionPause:
		if p.Position < 0 {
			room.mu.Unlock()
			c.sendError(s.logger, "invalid_position", "Position cannot be negative")
			return
		}
		if !validatePlaybackTrack(c, s.logger, room.State, &p) {
			room.mu.Unlock()
			return
		}
		p.Position = clampPlaybackPosition(p.Position)
		room.State.IsPlaying = false
		room.State.Position = p.Position
		room.State.LastUpdate = nowMs

	case ActionSeek:
		if p.Position < 0 {
			room.mu.Unlock()
			c.sendError(s.logger, "invalid_position", "Position cannot be negative")
			return
		}
		if !validatePlaybackTrack(c, s.logger, room.State, &p) {
			room.mu.Unlock()
			return
		}
		if room.State.IsPlaying {
			p.Position = compensateActionTransit(p.Position, p.CapturedAtServerTime, nowMs, rttMs)
		}
		p.Position = clampPlaybackPosition(p.Position)
		room.State.Position = p.Position
		room.State.LastUpdate = nowMs

	case ActionChangeTrack:
		if p.TrackInfo == nil {
			room.mu.Unlock()
			c.sendError(s.logger, "missing_track_info", "Track info is required for track change")
			return
		}
		if !sanitizeTrackInfo(p.TrackInfo) {
			room.mu.Unlock()
			c.sendError(s.logger, "invalid_track_info", "Track must have ID and title")
			return
		}
		// Everyone's player advances at the end of a song, so the same change arrives from each member.
		if cur := room.State.CurrentTrack; cur != nil && cur.ID == p.TrackInfo.ID && nowMs-room.TrackChangedAt < duplicateChangeWindow.Milliseconds() {
			room.mu.Unlock()
			return
		}
		room.TrackChangedAt = nowMs
		// Protobuf cannot tell an empty queue from an omitted one, so an empty
		// replacement needs the explicit replace_queue flag.
		queue := advanceQueue(room.State.Queue, p.TrackInfo.ID)
		if len(p.Queue) > 0 || p.ReplaceQueue {
			queue = sanitizeQueue(p.Queue)
			room.State.QueueTitle = p.QueueTitle
		}
		if queueWireSize(queue) > MaxQueueBytes {
			room.mu.Unlock()
			c.sendError(s.logger, "queue_too_large", "Queue is too large")
			return
		}
		room.State.Queue = queue
		room.State.CurrentTrack = cloneTrackInfo(p.TrackInfo)
		room.State.Position = 0
		room.State.IsPlaying = true // nobody sends a follow-up play; clients start as they buffer
		room.State.LastUpdate = nowMs
		p.TrackID = p.TrackInfo.ID
		p.Position = 0
		s.logger.Debug("Track changed", zap.String("room_code", room.Code), zap.String("track_id", p.TrackInfo.ID))

	case ActionSkipNext:
		if len(room.State.Queue) == 0 {
			room.mu.Unlock()
			return
		}
		// Rewritten as change_track so every client takes the one transition path.
		next := room.State.Queue[0]
		p = PlaybackActionPayload{Action: ActionChangeTrack, TrackInfo: &next, TrackID: next.ID}
		room.State.Queue = append([]TrackInfo(nil), room.State.Queue[1:]...)
		room.State.CurrentTrack = cloneTrackInfo(&next)
		room.State.Position, room.State.IsPlaying, room.State.LastUpdate = 0, true, nowMs

	case ActionSkipPrev:
		// ponytail: the server keeps no history, so previous restarts the track; add a history list if wanted.
		if room.State.CurrentTrack == nil {
			room.mu.Unlock()
			return
		}
		p = PlaybackActionPayload{Action: ActionSeek, TrackID: room.State.CurrentTrack.ID, Position: 0}
		room.State.Position, room.State.LastUpdate = 0, nowMs

	case ActionQueueAdd:
		if p.TrackInfo == nil {
			room.mu.Unlock()
			c.sendError(s.logger, "missing_track_info", "Track info is required for queue add")
			return
		}
		if !sanitizeTrackInfo(p.TrackInfo) {
			room.mu.Unlock()
			c.sendError(s.logger, "invalid_track_info", "Track must have ID and title")
			return
		}
		if len(room.State.Queue) >= MaxQueueSize {
			room.mu.Unlock()
			c.sendError(s.logger, "queue_full", "Queue is full")
			return
		}
		if (room.State.CurrentTrack != nil && room.State.CurrentTrack.ID == p.TrackInfo.ID) ||
			containsTrackID(room.State.Queue, p.TrackInfo.ID) {
			room.mu.Unlock()
			c.sendError(s.logger, "duplicate_track", "Track is already playing or queued")
			return
		}
		var queue []TrackInfo
		if p.InsertNext {
			queue = append([]TrackInfo{*p.TrackInfo}, room.State.Queue...)
		} else {
			queue = append(append([]TrackInfo(nil), room.State.Queue...), *p.TrackInfo)
		}
		if queueWireSize(queue) > MaxQueueBytes {
			room.mu.Unlock()
			c.sendError(s.logger, "queue_too_large", "Queue is too large")
			return
		}
		room.State.Queue = queue
		if room.State.CurrentTrack == nil {
			start := room.State.Queue[0]
			room.State.Queue = append([]TrackInfo(nil), room.State.Queue[1:]...)
			room.State.CurrentTrack = cloneTrackInfo(&start)
			room.State.Position, room.State.IsPlaying, room.State.LastUpdate = 0, true, nowMs
			p = PlaybackActionPayload{Action: ActionChangeTrack, TrackInfo: &start, TrackID: start.ID}
		}

	case ActionQueueRemove:
		if p.TrackID == "" {
			room.mu.Unlock()
			c.sendError(s.logger, "missing_track_id", "Track ID is required for queue remove")
			return
		}
		queue := make([]TrackInfo, 0, len(room.State.Queue))
		for _, track := range room.State.Queue {
			if track.ID != p.TrackID {
				queue = append(queue, track)
			}
		}
		room.State.Queue = queue

	case ActionQueueClear:
		room.State.Queue = []TrackInfo{}

	case ActionSyncQueue:
		queue := sanitizeQueue(p.Queue)
		if queueWireSize(queue) > MaxQueueBytes {
			room.mu.Unlock()
			c.sendError(s.logger, "queue_too_large", "Queue is too large")
			return
		}
		room.State.Queue = queue
		room.State.QueueTitle = p.QueueTitle

	case ActionUpdateTrack:
		// Metadata correction (e.g. a duration learned after loading) that keeps
		// position and play state, unlike change_track.
		if p.TrackInfo == nil || !sanitizeTrackInfo(p.TrackInfo) {
			room.mu.Unlock()
			c.sendError(s.logger, "invalid_track_info", "Track info is required")
			return
		}
		if room.State.CurrentTrack == nil || room.State.CurrentTrack.ID != p.TrackInfo.ID {
			room.mu.Unlock()
			c.sendError(s.logger, "stale_track", "Track update targets a stale track")
			return
		}
		room.State.CurrentTrack = cloneTrackInfo(p.TrackInfo)
		p.TrackID = p.TrackInfo.ID
		p.Position = livePlaybackPosition(room.State, nowMs)

	case ActionSetVolume:
		if math.IsNaN(p.Volume) || math.IsInf(p.Volume, 0) || p.Volume < 0 || p.Volume > 1 {
			room.mu.Unlock()
			c.sendError(s.logger, "invalid_volume", "Volume must be between 0 and 1")
			return
		}
		room.State.Volume = p.Volume

	default:
		room.mu.Unlock()
		c.sendError(s.logger, "unknown_action", fmt.Sprintf("Unknown action: %s", p.Action))
		return
	}
	if p.Action == ActionChangeTrack || p.Action == ActionQueueAdd || p.Action == ActionQueueRemove ||
		p.Action == ActionQueueClear || p.Action == ActionSyncQueue {
		p.Queue = append([]TrackInfo(nil), room.State.Queue...)
	}

	room.State.Revision++
	p.Revision = room.State.Revision
	if p.Action == ActionChangeTrack {
		room.TrackRevision = p.Revision
	}
	p.ServerTime = nowMs
	// The position is already compensated; forwarding the capture time would
	// invite clients to compensate twice.
	p.CapturedAtServerTime = 0
	p.HostSequence = 0
	p.OriginUserID = c.clientID()
	p.QueueTitle = room.State.QueueTitle
	clients := roomClientsLocked(room)
	if p.Action == ActionUpdateTrack {
		clients = withFeature(clients, FeatureUpdateTrack)
	}
	room.mu.Unlock()

	sendMessageToClients(s.logger, clients, MsgTypeSyncPlayback, p)
	s.logger.Debug("Playback action processed",
		zap.String("action", p.Action),
		zap.String("room_code", room.Code),
		zap.String("user", c.userName()),
		zap.Uint64("revision", p.Revision))
}

func (s *Server) handleBufferReady(c *Client, payload []byte) {
	var p BufferReadyPayload
	if err := decodePayload(payload, MsgTypeBufferReady, &p); err != nil {
		c.sendError(s.logger, "invalid_payload", "Invalid buffer ready payload")
		return
	}
	if p.TrackID == "" {
		c.sendError(s.logger, "missing_track_id", "Track ID is required")
		return
	}

	room := c.currentRoom()
	if room == nil {
		c.sendError(s.logger, "not_in_room", "You are not in a room")
		return
	}

	room.syncMu.Lock()
	defer room.syncMu.Unlock()
	room.mu.Lock()
	clientID := c.clientID()
	if room.Clients[clientID] != c {
		room.mu.Unlock()
		c.sendError(s.logger, "not_in_room", "You are not an active room member")
		return
	}
	if room.State.CurrentTrack == nil {
		room.mu.Unlock()
		c.sendError(s.logger, "stale_track", "Buffer readiness targets a stale track")
		return
	}
	if p.TrackID != room.State.CurrentTrack.ID || (p.TrackRevision != 0 && p.TrackRevision != room.TrackRevision) {
		room.mu.Unlock()
		c.sendError(s.logger, "stale_track", "Buffer readiness targets a stale track")
		return
	}

	nowMs := serverNowMs()
	position := livePlaybackPosition(room.State, nowMs)
	trackID := room.State.CurrentTrack.ID
	playing := room.State.IsPlaying
	revision := room.State.Revision
	room.mu.Unlock()

	c.sendMessage(s.logger, MsgTypeSyncPlayback, PlaybackActionPayload{
		Action: ActionSeek, TrackID: trackID, Position: position, ServerTime: nowMs, Revision: revision,
	})
	stateAction := ActionPause
	if playing {
		stateAction = ActionPlay
	}
	c.sendMessage(s.logger, MsgTypeSyncPlayback, PlaybackActionPayload{
		Action: stateAction, TrackID: trackID, Position: position, ServerTime: nowMs, Revision: revision,
	})
	c.reply(s.logger, MsgTypeBufferComplete, BufferCompletePayload{TrackID: trackID})
}

func (s *Server) handleRequestSync(c *Client) {
	room := c.currentRoom()
	if room == nil {
		c.sendError(s.logger, "not_in_room", "You are not in a room")
		return
	}

	room.syncMu.Lock()
	defer room.syncMu.Unlock()
	room.mu.RLock()
	if room.Clients[c.clientID()] != c {
		room.mu.RUnlock()
		c.sendError(s.logger, "not_in_room", "You are not an active room member")
		return
	}
	now := time.Now()
	if !c.allowSyncResponse(now) {
		room.mu.RUnlock()
		if c.hasFeature(FeatureRequestID) {
			c.sendError(s.logger, "sync_rate_limited", fmt.Sprintf("Retry after %d ms", SyncResponseInterval.Milliseconds()))
		}
		return
	}
	response := *syncSnapshotLocked(room, serverNowMs())
	position := response.Position
	room.mu.RUnlock()

	s.logger.Debug("Sync request received",
		zap.String("username", c.userName()),
		zap.String("user_id", c.clientID()),
		zap.Bool("response_playing", response.IsPlaying),
		zap.Int64("position", position),
		zap.Uint64("revision", response.Revision))
	c.reply(s.logger, MsgTypeSyncState, response)
}

func withFeature(clients []*Client, feature string) []*Client {
	kept := clients[:0]
	for _, client := range clients {
		if client.hasFeature(feature) {
			kept = append(kept, client)
		}
	}
	return kept
}
