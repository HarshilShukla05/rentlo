import L from 'leaflet';
import 'leaflet/dist/leaflet.css';
import './styles.css';

const ui = Object.fromEntries([
  'whatsapp', 'leave', 'city', 'profile', 'filters', 'areas', 'clear-area', 'matches', 'matches-panel',
  'map', 'map-label', 'map-caption', 'map-error', 'my-area', 'reset-map', 'intro-copy', 'notice', 'connection',
].map(id => [id, document.getElementById(id)]));

const intents = {
  has_room: { label: 'Room available', activity: 'with a room' },
  needs_room: { label: 'Looking for a room', activity: 'seeking a room' },
  team_up: { label: 'Open to looking together', activity: 'open to teaming up' },
};
const placeholder = { name: 'Hyderabad', point: { lat: 17.418, lng: 78.375 } };
const state = {
  city: 'Hyderabad', cities: [placeholder], areas: [], activity: [], matches: [], me: null,
  filter: 'all', selected: null, whatsapp: null, busy: false, cityRequest: 0, refreshRequest: 0,
};

const currency = new Intl.NumberFormat('en-IN', { style: 'currency', currency: 'INR', maximumFractionDigits: 0 });
const moveIn = new Intl.DateTimeFormat('en-IN', { day: 'numeric', month: 'short', timeZone: 'UTC' });
const element = (tag, text = '', className = '') => {
  const item = document.createElement(tag);
  if (text) item.textContent = text;
  if (className) item.className = className;
  return item;
};
const button = (text, className, onClick) => {
  const control = element('button', text, className);
  control.type = 'button';
  control.addEventListener('click', onClick);
  return control;
};

const pathToken = location.pathname.startsWith('/m/') ? location.pathname.slice(3) : null;
const access = new URLSearchParams(location.hash.slice(1)).get('access') ?? pathToken;
if (access !== null) history.replaceState(null, '', '/');

function api(path, method = 'GET', body) {
  const headers = body === undefined ? {} : { 'Content-Type': 'application/json' };
  return fetch(`/api/v1${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) })
    .then(async response => {
      if (!response.ok) {
        const detail = await response.json().catch(() => ({}));
        throw Object.assign(new Error(detail.message ?? 'Rentlo could not complete the request.'), { status: response.status });
      }
      return response.status === 204 ? null : response.json();
    });
}

let map;
let areaMarkers = [];
let ownMarker;
try {
  map = L.map(ui.map, {
    center: [placeholder.point.lat, placeholder.point.lng],
    zoom: 11.3, minZoom: 4, maxZoom: 16, zoomSnap: 0.25,
    zoomControl: false, scrollWheelZoom: false,
  });
  map.attributionControl.setPrefix(false);
  let tilesLoaded = false;
  const tiles = L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
    maxZoom: 16,
    keepBuffer: 1,
    referrerPolicy: 'origin',
    attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
  });
  tiles.on('tileload', () => {
    tilesLoaded = true;
    ui['map-error'].hidden = true;
  });
  tiles.on('tileerror', () => {
    if (!tilesLoaded) ui['map-error'].hidden = false;
  });
  tiles.addTo(map);
} catch {
  map = undefined;
  ui['map-error'].hidden = false;
}
for (const control of document.querySelectorAll('[data-zoom]')) {
  control.disabled = !map;
  control.addEventListener('click', () => map?.setZoom(map.getZoom() + Number(control.dataset.zoom)));
}

function showNotice(text) {
  ui.notice.textContent = text;
  ui.notice.hidden = !text;
}

function placeOf(name) {
  return state.cities.find(place => place.name === name);
}

function moveMap(position, zoom) {
  if (!map || !position) return;
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  map.setView([position.lat, position.lng], zoom, { animate: !reduced });
}

function focusCity() {
  const places = state.areas;
  if (map && places.length > 1) {
    const bounds = L.latLngBounds(places.map(({ point }) => [point.lat, point.lng]));
    map.fitBounds(bounds, { padding: [56, 72], maxZoom: 12.25, animate: false });
  } else {
    moveMap(placeOf(state.city)?.point ?? state.me?.location?.point, 11.3);
  }
}

function selectedArea(name) {
  state.selected = name;
  renderAreas();
  renderMatches();
  const area = state.areas.find(area => area.name === name);
  if (area) moveMap(area.point, 13);
  else focusCity();
}

function focusMine() {
  if (!state.me?.location) return;
  const city = state.me.card.city;
  const focus = () => moveMap(state.me.location.point, state.me.location.precision === 'city' ? 11.3 : 13);
  if (state.city !== city) {
    loadCity(city, false).then(focus).catch(error => showNotice(error.message));
  } else {
    focus();
  }
}

ui['my-area'].addEventListener('click', focusMine);
ui['reset-map'].addEventListener('click', () => selectedArea(null));
ui['clear-area'].addEventListener('click', () => selectedArea(null));
ui.city.addEventListener('change', event => loadCity(event.target.value).catch(error => showNotice(error.message)));
ui.filters.addEventListener('click', event => {
  const control = event.target.closest('[data-filter]');
  if (!control) return;
  state.filter = control.dataset.filter;
  for (const sibling of ui.filters.querySelectorAll('button')) sibling.setAttribute('aria-pressed', String(sibling === control));
  renderAreas();
  renderMatches();
});

function shownActivity(name) {
  return state.activity.filter(item => item.area === name && (state.filter === 'all' || item.intent === state.filter));
}
const activityLine = items => items.map(item => `${item.at_least}+ ${intents[item.intent].activity}`).join(' · ');

function renderAreas() {
  const areaList = [...state.areas].sort((a, b) =>
    Number(shownActivity(b.name).length > 0) - Number(shownActivity(a.name).length > 0));
  const rows = [];
  const citywide = shownActivity(null);
  if (citywide.length) {
    rows.push(element('p', `Citywide · ${activityLine(citywide)}`, 'soft-note'));
  }
  for (const area of areaList) {
    const item = button('', 'area-row', () => selectedArea(area.name));
    item.setAttribute('aria-current', String(state.selected === area.name));
    const label = element('span');
    label.append(element('strong', area.name));
    const summary = activityLine(shownActivity(area.name));
    if (summary) label.append(element('small', summary));
    const arrow = element('span', '', 'arrow');
    arrow.setAttribute('aria-hidden', 'true');
    item.append(label, arrow);
    rows.push(item);
  }
  if (!areaList.length) rows.push(element('p', 'Use the map to explore this city.', 'soft-note'));
  ui.areas.replaceChildren(...rows);
  ui['map-label'].textContent = state.selected ?? state.city;
  ui['map-caption'].textContent = state.selected ? state.city : 'Neighbourhood view';
  ui['clear-area'].hidden = !state.selected;
  renderMarkers(areaList);
}

function mapMarker(point, label, own = false) {
  const icon = L.divIcon({ html: label, className: 'area-marker-wrap', iconSize: [0, 0] });
  return L.marker([point.lat, point.lng], { icon, keyboard: false, interactive: !own, zIndexOffset: own ? 1000 : 0 }).addTo(map);
}

function renderMarkers(areas) {
  if (!map) return;
  for (const marker of areaMarkers) marker.remove();
  areaMarkers = areas.map(area => {
    const activity = shownActivity(area.name);
    const label = button(area.name, `area-marker${activity.length ? ' has-activity' : ''}`, () => {
      selectedArea(area.name);
      areaMarkers.find(marker => marker.getElement().textContent === area.name)?.getElement().querySelector('button').focus({ preventScroll: true });
    });
    label.dataset.intent = activity.length === 1 ? activity[0].intent : state.filter;
    label.setAttribute('aria-pressed', String(area.name === state.selected));
    if (activity.length) label.setAttribute('aria-label', `${area.name}, ${activityLine(activity)}`);
    return mapMarker(area.point, label);
  });
  ownMarker?.remove();
  if (state.me?.location && state.me.card?.city === state.city) {
    const { point, precision } = state.me.location;
    ownMarker = mapMarker(point, element('span', precision === 'city' ? 'Your city' : 'Your area', 'own-marker'), true);
  }
}

function renderProfile() {
  const card = state.me?.card;
  const member = Boolean(card);
  document.querySelector('.sidebar').classList.toggle('member', member);
  ui.profile.hidden = !member;
  ui['matches-panel'].hidden = !member;
  ui['my-area'].hidden = !state.me?.location || !map;
  ui.whatsapp.hidden = !state.whatsapp || member;
  ui.leave.hidden = !member;
  ui.connection.textContent = member ? 'Verified through WhatsApp' : 'Neighbourhood map';
  ui['intro-copy'].textContent = member
    ? 'Your approximate area and the people who fit how you live.'
    : 'Explore neighbourhoods to find where you’d like to share a home.';
  if (!card) return;
  const name = [card.area, card.city].filter(Boolean).join(', ');
  const locationNote = !state.me.location || state.me.location.precision === 'city'
    ? 'Send a neighbourhood name or area pin in WhatsApp to refine the map.'
    : 'Approximate area';
  ui.profile.replaceChildren(
    element('span', state.me.paused ? 'Matching paused' : 'Live for matching', 'profile-status'),
    button(state.me.paused ? 'Go live' : 'Pause', 'button button-outline', () => action(async () => {
      await api('/me/participation', 'PUT', { paused: !state.me.paused });
      await refresh();
    })),
    element('div'),
  );
  ui.profile.lastElementChild.append(element('h2', name), element('p', locationNote));
}

function whatsappLink(url, text) {
  const target = new URL(url);
  if (target.origin !== 'https://wa.me') throw new Error('WhatsApp link is unavailable.');
  if (text) target.searchParams.set('text', text);
  return target.href;
}

function currencyRange(card) {
  const max = currency.format(card.budget_max);
  if (card.intent === 'has_room') return `${max} / month`;
  return card.budget_min ? `${currency.format(card.budget_min)}–${max} / month` : `Up to ${max} / month`;
}

function renderMatches() {
  if (!state.me?.card) {
    ui.matches.replaceChildren();
    return;
  }
  if (state.me.paused) {
    ui.matches.replaceChildren(element('p', 'Go live when you’re ready to see matches.', 'soft-note'));
    return;
  }
  const rows = state.matches.filter(match =>
    (state.filter === 'all' || match.card.intent === state.filter)
    && (!state.selected || state.city !== state.me.card.city || match.card.area === state.selected));
  if (!rows.length) {
    ui.matches.replaceChildren(element('p',
      state.filter !== 'all' || state.selected ? 'No matches in this view.' : 'Your best matches will appear here as people join.', 'soft-note'));
    return;
  }
  ui.matches.replaceChildren(...rows.slice(0, 6).map(match => {
    const { card } = match;
    const item = element('article', '', 'match-card');
    item.append(element('h3', [intents[card.intent].label, card.area || card.city].join(' · ')));
    const date = card.move_in ? ` · From ${moveIn.format(new Date(card.move_in))}` : '';
    item.append(element('p', currencyRange(card) + date, 'budget'));
    const tags = element('div', '', 'tags');
    for (const note of match.fits) tags.append(element('span', note, 'tag'));
    for (const note of match.watch) tags.append(element('span', note, 'tag watch'));
    if (tags.children.length) item.append(tags);
    const actions = element('div', '', 'match-actions');
    if (match.whatsapp) {
      const chat = element('a', match.name ? `Message ${match.name}` : 'Open WhatsApp', 'button button-dark');
      chat.href = whatsappLink(match.whatsapp);
      actions.append(chat);
    } else if (match.status === 'mutual') {
      actions.append(element('span', 'You both said yes. Check WhatsApp for your introduction.', 'soft-note'));
    } else if (!match.decided) {
      for (const accept of [true, false]) {
        actions.append(button(accept ? 'Interested' : 'Skip', `button ${accept ? 'button-dark' : 'button-outline'}`, () =>
          action(async () => {
            const result = await api(`/me/matches/${encodeURIComponent(match.id)}/decision`, 'POST', { accept });
            if (result === 'liked') showNotice('Interest sent. You’ll be connected if they say yes too.');
            await refresh();
          })));
      }
    } else {
      actions.append(element('span', 'Waiting for their reply', 'soft-note'));
    }
    item.append(actions);
    return item;
  }));
}

async function loadCity(city, move = true) {
  const version = ++state.cityRequest;
  const query = `?city=${encodeURIComponent(city)}`;
  const [data, activity] = await Promise.all([
    api(`/places${query}`),
    state.me?.card ? api(`/map/activity${query}`).catch(error => {
      if (error.status === 401) return [];
      throw error;
    }) : [],
  ]);
  if (version !== state.cityRequest) return;
  state.city = city;
  state.cities = data.cities;
  state.areas = data.areas;
  state.activity = state.me?.card ? activity : [];
  state.whatsapp = data.whatsapp;
  state.selected = null;
  const names = [...new Set([city, ...data.cities.map(place => place.name)])].sort();
  ui.city.replaceChildren(...names.map(name => element('option', name)));
  ui.city.value = city;
  if (data.whatsapp) ui.whatsapp.href = whatsappLink(data.whatsapp, access === null ? 'Hi' : 'link');
  renderProfile();
  renderAreas();
  renderMatches();
  if (move) focusCity();
}

async function refresh() {
  const version = ++state.refreshRequest;
  const me = await api('/me').catch(error => {
    if (error.status === 401) return null;
    throw error;
  });
  if (version !== state.refreshRequest) return;
  const wasMember = Boolean(state.me?.card);
  if (!me?.card) {
    state.me = null;
    state.matches = [];
    state.activity = [];
    if (wasMember) showNotice('Your private map session ended. Send link in WhatsApp to open it again.');
  } else {
    const city = state.city;
    const [matches, activity] = await Promise.all([
      api('/me/matches'),
      api(`/map/activity?city=${encodeURIComponent(city)}`),
    ]);
    if (version !== state.refreshRequest) return;
    state.me = me;
    state.matches = matches;
    if (state.city === city) state.activity = activity;
  }
  renderProfile();
  renderAreas();
  renderMatches();
}

async function action(run) {
  if (state.busy) return;
  state.busy = true;
  try {
    showNotice('');
    await run();
  } catch (error) {
    showNotice(error.message);
  } finally {
    state.busy = false;
  }
}

ui.leave.addEventListener('click', () => action(async () => {
  await api('/session/sign-out', 'POST', {});
  await refresh();
  showNotice('');
}));

async function start() {
  if (access !== null) {
    try {
      await api('/session', 'POST', { token: access });
    } catch {
      showNotice('This link is expired or has been used. Send link in WhatsApp for a fresh one.');
    }
  }
  try {
    const me = await api('/me');
    if (me.card) {
      state.me = me;
      state.matches = await api('/me/matches');
    }
  } catch (error) {
    if (error.status !== 401) showNotice(error.message);
  }
  await loadCity(state.me?.card?.city ?? state.city, !state.me?.location);
  if (state.me?.location) focusMine();
  setInterval(() => {
    if (!document.hidden && state.me?.card && !state.busy) refresh().catch(error => showNotice(error.message));
  }, 30_000);
}

start().catch(error => showNotice(error.message));
