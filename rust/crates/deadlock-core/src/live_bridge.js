"use strict";

// Sigil live bridge.
//
// The game's top bar loads this once per team panel. Every two seconds it reads what the top bar
// already shows for each player (name, hero, souls, level, kills, deaths, assists, objective
// damage, alive or dead) and the match clock, and prints one line to the console when anything
// changed. With -condebug the console is written to console.log, which Sigil reads. Nothing is
// drawn, sent or stored by this script.
//
// The wire format is read by deadlock_core::live_match. Change both together and bump VERSION.

(function () {
  var VERSION = 1;
  var TAG = "[SIGIL-LIVE]";
  var ROSTER_TAG = "[SIGIL-LIVE-ROSTER]";
  var EVERY = 2;
  // A board that did not change is printed again this often, so Sigil can tell a quiet match
  // from a bridge that stopped.
  var REPEAT = 20;

  var team = $.GetContextPanel();
  var printed = "";
  var printedAgo = 0;
  var printedRoster = "";

  // A dialog variable as a panel and its parents see it, or null when it does not resolve.
  function variable(name, panel) {
    try {
      var value = $.Localize("{" + name + "}", panel);
      if (typeof value !== "string" || value.length === 0 || value.indexOf("{") >= 0) {
        return null;
      }
      return value;
    } catch (e) {
      return null;
    }
  }

  function number(names, panel) {
    for (var i = 0; i < names.length; i++) {
      var value = variable(names[i], panel);
      if (value === null) continue;
      var parsed = parseInt(value.replace(/[^0-9-]/g, ""), 10);
      if (!isNaN(parsed)) return parsed;
    }
    return null;
  }

  function kids(panel) {
    try {
      return panel && panel.Children ? panel.Children() : [];
    } catch (e) {
      return [];
    }
  }

  function has(panel, name) {
    try {
      return !!(panel.BHasClass && panel.BHasClass(name));
    } catch (e) {
      return false;
    }
  }

  function top(panel) {
    var at = panel;
    while (at && at.GetParent && at.GetParent()) at = at.GetParent();
    return at;
  }

  function labelText(panel) {
    if (!panel) return null;
    if (typeof panel.text === "string" && panel.text.length > 0 && panel.text.indexOf("{") < 0) {
      return panel.text.replace(/<[^>]*>/g, "");
    }
    var children = kids(panel);
    for (var i = 0; i < children.length; i++) {
      var found = labelText(children[i]);
      if (found !== null) return found;
    }
    return null;
  }

  // The clock lives on the top bar itself, above the team panels.
  var clockPanel = null;
  function clock() {
    var direct = variable("s:game_clock", team);
    if (direct !== null) return direct;
    if (clockPanel && clockPanel.IsValid && !clockPanel.IsValid()) clockPanel = null;
    if (!clockPanel) {
      var bar = team;
      while (bar && bar.paneltype !== "CitadelHudTopBar" && bar.GetParent) bar = bar.GetParent();
      if (bar && bar.FindChildTraverse) clockPanel = bar.FindChildTraverse("GameTime");
    }
    if (!clockPanel) return null;
    return variable("s:game_clock", clockPanel) || labelText(clockPanel);
  }

  // Player panels: the first panel down each branch that resolves a player name. Labels inside
  // it resolve the same variable through their parents, so the walk stops there.
  function players(panel, out, depth) {
    if (depth > 12) return;
    var children = kids(panel);
    for (var i = 0; i < children.length; i++) {
      if (variable("s:player_name", children[i]) !== null) {
        out.push(children[i]);
      } else {
        players(children[i], out, depth + 1);
      }
    }
  }

  function player(panel) {
    return {
      name: variable("s:player_name", panel),
      hero: variable("s:hero_name", panel),
      hero_id: number(["i:hero_id", "d:hero_id"], panel),
      souls: variable("s:gold", panel),
      level: number(["i:player_level"], panel),
      k: number(["i:kills"], panel),
      d: number(["i:deaths"], panel),
      a: number(["i:assists"], panel),
      obj: number(["i:objective_damage"], panel),
      me: has(panel, "LocalPlayer"),
      dead: has(panel, "Dead"),
    };
  }

  function side() {
    if (team.id === "TeamFriendly") return "friendly";
    if (team.id === "TeamEnemy") return "enemy";
    return null;
  }

  // The versus screen before the match shows every player with an avatar: the one place a name
  // meets an account. The property the avatar keeps it in is not documented, so a few are tried.
  var ACCOUNT_KEYS = ["steamid", "accountid", "steamId", "accountId", "xuid"];
  function account(avatar) {
    for (var i = 0; i < ACCOUNT_KEYS.length; i++) {
      var key = ACCOUNT_KEYS[i];
      try {
        var value = avatar[key];
        if (value !== undefined && value !== null && value !== "" && value !== 0) return String(value);
      } catch (e) {}
      try {
        var attribute = avatar.GetAttributeString ? avatar.GetAttributeString(key, "") : "";
        if (attribute) return attribute;
      } catch (e) {}
    }
    return null;
  }

  function child(panel, id) {
    try {
      return panel && panel.FindChildTraverse ? panel.FindChildTraverse(id) : null;
    } catch (e) {
      return null;
    }
  }

  function roster() {
    var root = top(team);
    if (!root) return;
    var teams = [];
    var lists = ["Team1Players", "Team2Players"];
    for (var t = 0; t < lists.length; t++) {
      var list = child(root, lists[t]);
      if (!list) continue;
      var entries = kids(list);
      var rows = [];
      for (var i = 0; i < entries.length; i++) {
        var avatar = child(entries[i], "AvatarImage");
        rows.push({ name: labelText(child(entries[i], "UserName")), id: avatar ? account(avatar) : null });
      }
      teams.push(rows);
    }
    if (teams.length === 0) return;
    var line = JSON.stringify({ v: VERSION, teams: teams });
    if (line !== printedRoster) {
      printedRoster = line;
      $.Msg(ROSTER_TAG + " " + line);
    }
  }

  function report() {
    var which = side();
    if (which === null) return;
    printedAgo += EVERY;
    var found = [];
    players(team, found, 0);
    if (found.length > 0) {
      var rows = [];
      for (var i = 0; i < found.length; i++) rows.push(player(found[i]));
      var line = JSON.stringify({ v: VERSION, side: which, clock: clock(), players: rows });
      if (line !== printed || printedAgo >= REPEAT) {
        printed = line;
        printedAgo = 0;
        $.Msg(TAG + " " + line);
      }
    }
    // Both panels run this script; one roster is enough.
    if (which === "friendly") roster();
  }

  // A panel can vanish between two reads. That costs one report, never the match.
  function tick() {
    if (!team || (team.IsValid && !team.IsValid())) return;
    try {
      report();
    } catch (e) {}
    $.Schedule(EVERY, tick);
  }

  $.Schedule(EVERY, tick);
})();
