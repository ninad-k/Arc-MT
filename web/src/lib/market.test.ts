import { describe, expect, it } from 'vitest';
import type { MarketSetup, MarketViewDto } from './api';
import {
	biasInfo,
	biasSentence,
	biasVerdict,
	canPrepare,
	cleanSymbolInput,
	clampConfidence,
	confidenceWord,
	eventMinutes,
	expiryText,
	fmtPercent,
	fmtPoints,
	importanceInfo,
	isExpired,
	macdSign,
	minutesAwayText,
	newsLabel,
	newsTone,
	outcomeInfo,
	percent,
	prepareChoices,
	safeHttpUrl,
	setupLine,
	summarizeStats,
	timeAgo,
	toolLabel,
	topHeadlines,
	trendArrow,
	validateVolume,
	viewDigits
} from './market';

const NOW = Date.parse('2026-10-30T13:50:00Z');
const at = (min: number) => new Date(NOW + min * 60_000).toISOString();

const setup: MarketSetup = {
	side: 'buy',
	entry_low: 3921.4,
	entry_high: 3925.22,
	stop_loss: 3911,
	take_profit: 3944.1,
	take_profit_2: 3958,
	risk_reward: 1.5,
	atr_h1: 12.4,
	atr_m15: 5.1
};

function view(over: Partial<MarketViewDto> = {}): MarketViewDto {
	return {
		id: 'v1',
		symbol: 'XAUUSD',
		broker_symbol: 'XAUUSD_',
		generated_at: at(0),
		valid_until: at(240),
		price: { bid: 3925.09, ask: 3925.22 },
		bias: 'bullish',
		confidence: 62,
		score: 62,
		stand_aside: false,
		stand_aside_reason: null,
		setup,
		timeframes: [],
		levels: {
			pivot: 3918.5,
			r1: 3930,
			r2: 3941.25,
			s1: 3905,
			s2: 3899,
			prev_day_high: 3935,
			prev_day_low: 3898,
			vwap: 3920.1,
			support: [3912],
			resistance: [3940]
		},
		factors: [],
		events: [],
		news: { configured: false, score: null, label: null, count: 0, headlines: [] },
		summary: '',
		disclaimer: '',
		...over
	};
}

describe('bias', () => {
	it('labels and colours each bias', () => {
		expect(biasInfo('bullish')).toMatchObject({ label: 'Bullish', tone: 'up' });
		expect(biasInfo('bearish')).toMatchObject({ label: 'Bearish', tone: 'down' });
		expect(biasInfo('neutral')).toMatchObject({ label: 'Neutral', tone: 'flat' });
		expect(biasInfo('bullish').pill).toContain('text-up');
		expect(biasInfo('bearish').pill).toContain('text-down');
		expect(biasInfo('neutral').pill).toContain('text-mute');
	});

	it('treats an unknown bias as neutral', () => {
		expect(biasInfo('sideways').label).toBe('Neutral');
	});

	it('draws arrows for trends', () => {
		expect(trendArrow('up')).toBe('↑');
		expect(trendArrow('down')).toBe('↓');
		expect(trendArrow('flat')).toBe('→');
		expect(trendArrow('')).toBe('→');
	});
});

describe('confidence', () => {
	it('uses plain words at the band edges', () => {
		expect(confidenceWord(100)).toBe('high');
		expect(confidenceWord(70)).toBe('high');
		expect(confidenceWord(69)).toBe('moderate');
		expect(confidenceWord(50)).toBe('moderate');
		expect(confidenceWord(49)).toBe('low');
		expect(confidenceWord(30)).toBe('low');
		expect(confidenceWord(29)).toBe('very low');
		expect(confidenceWord(0)).toBe('very low');
	});

	it('clamps and rounds what the meter draws', () => {
		expect(clampConfidence(-5)).toBe(0);
		expect(clampConfidence(140)).toBe(100);
		expect(clampConfidence(61.6)).toBe(62);
		expect(clampConfidence(Number.NaN)).toBe(0);
	});

	it('reads as one sentence', () => {
		expect(biasSentence('bullish', 62)).toBe('Bullish, moderate confidence (62 of 100)');
		expect(biasSentence('neutral', 12)).toBe('Neutral, very low confidence (12 of 100)');
	});
});

describe('indicator readings', () => {
	it('shows the sign of the MACD histogram', () => {
		expect(macdSign(0.4)).toMatchObject({ text: '+', tone: 'up' });
		expect(macdSign(-0.01)).toMatchObject({ text: '−', tone: 'down' });
		expect(macdSign(0)).toMatchObject({ text: '0', tone: 'flat' });
	});

	it('writes factor points with a sign', () => {
		expect(fmtPoints(25)).toBe('+25');
		expect(fmtPoints(-10)).toBe('−10');
		expect(fmtPoints(12.5)).toBe('+12.5');
		expect(fmtPoints(0)).toBe('0');
	});
});

describe('price digits', () => {
	it('uses the digits the server sends', () => {
		expect(viewDigits(view({ digits: 3 }))).toBe(3);
	});

	it('then the watchlist hint', () => {
		expect(viewDigits(view(), 2)).toBe(2);
	});

	it('infers the longest number otherwise', () => {
		expect(viewDigits(view())).toBe(2);
		const eur = view({
			price: { bid: 1.08432, ask: 1.08441 },
			setup: { ...setup, entry_low: 1.0841, entry_high: 1.08441, stop_loss: 1.0802, take_profit: 1.0902, take_profit_2: null },
			levels: { ...view().levels, pivot: 1.0845, r1: 1.0870, r2: null, s1: 1.082, s2: null, support: [], resistance: [] }
		});
		expect(viewDigits(eur)).toBe(5);
	});

	it('never goes below the usual decimals for a price of that size', () => {
		const round = view({
			price: { bid: 3925, ask: 3926 },
			setup: null,
			levels: { ...view().levels, pivot: 3920, r1: 3930, r2: 3940, s1: 3910, s2: 3900, prev_day_high: 3935, prev_day_low: 3898, vwap: 3921, support: [3912], resistance: [3940] }
		});
		expect(viewDigits(round)).toBe(2);
		const fx = view({
			price: { bid: 1.08, ask: 1.09 },
			setup: null,
			levels: { ...view().levels, pivot: 1.08, r1: null, r2: null, s1: null, s2: null, prev_day_high: null, prev_day_low: null, vwap: null, support: [], resistance: [] }
		});
		expect(viewDigits(fx)).toBe(4);
	});
});

describe('expiry', () => {
	it('says how long the view is still valid', () => {
		expect(expiryText(at(192), NOW)).toBe('expires in 3 h 12 min');
		expect(expiryText(at(180), NOW)).toBe('expires in 3 h');
		expect(expiryText(at(45), NOW)).toBe('expires in 45 min');
		expect(expiryText(at(0.2), NOW)).toBe('expires in 1 min');
	});

	it('says when it has expired', () => {
		expect(expiryText(at(-5), NOW)).toBe('expired 5 min ago');
		expect(expiryText(at(-125), NOW)).toBe('expired 2 h 5 min ago');
		expect(expiryText(at(-0.2), NOW)).toBe('expired just now');
		expect(expiryText('nonsense', NOW)).toBe('validity unknown');
	});

	it('knows when it is over', () => {
		expect(isExpired(at(1), NOW)).toBe(false);
		expect(isExpired(at(0), NOW)).toBe(true);
		expect(isExpired(at(-1), NOW)).toBe(true);
		expect(isExpired('nonsense', NOW)).toBe(false);
	});
});

describe('minutes away', () => {
	it('words the future and the past', () => {
		expect(minutesAwayText(40)).toBe('in 40 min');
		expect(minutesAwayText(125)).toBe('in 2 h 5 min');
		expect(minutesAwayText(60)).toBe('in 1 h');
		expect(minutesAwayText(0)).toBe('now');
		expect(minutesAwayText(-12)).toBe('12 min ago');
		expect(minutesAwayText(-90)).toBe('1 h 30 min ago');
	});

	it('works from the event time, so a view that has aged stays right', () => {
		expect(eventMinutes({ time: at(40), minutes_away: 60 }, NOW)).toBe(40);
		expect(eventMinutes({ time: at(40), minutes_away: 60 }, NOW + 10 * 60_000)).toBe(30);
		expect(eventMinutes({ time: 'bad', minutes_away: 7 }, NOW)).toBe(7);
	});

	it('words the age of a headline', () => {
		expect(timeAgo(at(-0.5), NOW)).toBe('just now');
		expect(timeAgo(at(-35), NOW)).toBe('35 min ago');
		expect(timeAgo(at(-180), NOW)).toBe('3 h ago');
		expect(timeAgo(at(-60 * 50), NOW)).toBe('2 d ago');
		expect(timeAgo('bad', NOW)).toBe('');
	});

	it('names the importance', () => {
		expect(importanceInfo('high').label).toBe('High');
		expect(importanceInfo('moderate').label).toBe('Medium');
		expect(importanceInfo('mystery').label).toBe('mystery');
	});
});

describe('news', () => {
	it('labels each state', () => {
		expect(newsLabel({ configured: false, label: null, count: 0 })).toBe('Not configured');
		expect(newsLabel({ configured: true, label: null, count: 0 })).toBe('No relevant headlines');
		expect(newsLabel({ configured: true, label: 'slightly positive', count: 7 })).toBe('Slightly positive (7 headlines)');
		expect(newsLabel({ configured: true, label: 'negative', count: 1 })).toBe('Negative (1 headline)');
	});

	it('picks a tone from the score', () => {
		expect(newsTone(0.2)).toBe('up');
		expect(newsTone(-0.2)).toBe('down');
		expect(newsTone(0.05)).toBe('flat');
		expect(newsTone(null)).toBe('flat');
	});

	it('only links http and https', () => {
		expect(safeHttpUrl('https://example.com/a?b=1')).toBe('https://example.com/a?b=1');
		expect(safeHttpUrl('http://example.com/')).toBe('http://example.com/');
		expect(safeHttpUrl('javascript:alert(1)')).toBeNull();
		expect(safeHttpUrl('data:text/html,hi')).toBeNull();
		expect(safeHttpUrl('not a url')).toBeNull();
	});

	it('keeps the five newest headlines', () => {
		const list = Array.from({ length: 8 }, (_, i) => ({
			title: `h${i}`,
			source: 's',
			url: 'https://example.com',
			published_at: at(-i * 10),
			tone: 0
		}));
		const top = topHeadlines([...list].reverse());
		expect(top.map((h) => h.title)).toEqual(['h0', 'h1', 'h2', 'h3', 'h4']);
		expect(topHeadlines([], 5)).toEqual([]);
	});
});

describe('journal', () => {
	it('labels every outcome', () => {
		expect(outcomeInfo('tp_hit').label).toBe('TP hit');
		expect(outcomeInfo('sl_hit').label).toBe('SL hit');
		expect(outcomeInfo('expired').label).toBe('Expired');
		expect(outcomeInfo('open').label).toBe('Open');
		expect(outcomeInfo('no_setup').label).toBe('No setup');
		expect(outcomeInfo('weird').label).toBe('weird');
	});

	it('says what the bias check found', () => {
		expect(biasVerdict(true)).toBe('Bias right');
		expect(biasVerdict(false)).toBe('Bias wrong');
		expect(biasVerdict(null)).toBe('Not scored yet');
	});

	it('guards the division', () => {
		expect(percent(1, 0)).toBeNull();
		expect(percent(0, 0)).toBeNull();
		expect(percent(1, 3)).toBe(33);
		expect(percent(2, 3)).toBe(67);
		expect(percent(5, 5)).toBe(100);
		expect(fmtPercent(null)).toBe('—');
		expect(fmtPercent(40)).toBe('40%');
	});

	it('works out the track record from the counts', () => {
		const t = summarizeStats({ total: 20, open: 2, tp_hit: 6, sl_hit: 3, expired: 1, no_setup: 8, bias_correct: 9, bias_wrong: 6 });
		expect(t.setups).toBe(12);
		expect(t.settled).toBe(10);
		expect(t.tpRate).toBe(60);
		expect(t.slRate).toBe(30);
		expect(t.expiredRate).toBe(10);
		expect(t.biasAccuracy).toBe(60);
		expect(t.noSetup).toBe(8);
	});

	it('shows dashes, not NaN, when nothing is settled', () => {
		const t = summarizeStats({});
		expect(t.tpRate).toBeNull();
		expect(t.biasAccuracy).toBeNull();
		expect(summarizeStats(null).setups).toBe(0);
		const onlyOpen = summarizeStats({ open: 3, no_setup: 2 });
		expect(onlyOpen.setups).toBe(3);
		expect(onlyOpen.tpRate).toBeNull();
	});

	it('ignores nonsense counts', () => {
		const t = summarizeStats({ tp_hit: -2, sl_hit: Number.NaN, expired: 2.9 });
		expect(t.tpHit).toBe(0);
		expect(t.slHit).toBe(0);
		expect(t.expired).toBe(2);
	});
});

describe('validateVolume', () => {
	it('requires a value', () => {
		expect(validateVolume('')).toEqual({ ok: false, error: 'Enter a volume in lots.' });
		expect(validateVolume('   ').ok).toBe(false);
	});

	it('accepts normal lot sizes', () => {
		expect(validateVolume('0.01')).toEqual({ ok: true, value: 0.01 });
		expect(validateVolume(' 0.5 ')).toEqual({ ok: true, value: 0.5 });
		expect(validateVolume('1')).toEqual({ ok: true, value: 1 });
		expect(validateVolume('2.50')).toEqual({ ok: true, value: 2.5 });
		expect(validateVolume('.25')).toEqual({ ok: true, value: 0.25 });
		expect(validateVolume('10.')).toEqual({ ok: true, value: 10 });
	});

	it('refuses zero and negatives', () => {
		expect(validateVolume('0').ok).toBe(false);
		expect(validateVolume('0.00').ok).toBe(false);
		expect(validateVolume('-1')).toEqual({ ok: false, error: 'Volume must be greater than 0.' });
	});

	it('refuses more than two decimals', () => {
		expect(validateVolume('0.001')).toEqual({ ok: false, error: 'Use at most 2 decimals.' });
		expect(validateVolume('1.234').ok).toBe(false);
	});

	it('refuses anything that is not a plain number', () => {
		for (const bad of ['abc', '1e-2', '0,01', '1.2.3', '+1', '.', '0x10', 'Infinity', '1 2']) {
			expect(validateVolume(bad).ok, bad).toBe(false);
		}
	});
});

describe('prepare trade', () => {
	it('shows the price each order type would use for a buy', () => {
		const [market, limit] = prepareChoices(setup, { bid: 3925.09, ask: 3925.22 });
		expect(market).toMatchObject({ type: 'market', price: 3925.22 });
		expect(limit).toMatchObject({ type: 'limit', price: 3921.4 });
	});

	it('and for a sell', () => {
		const sell = { ...setup, side: 'sell' as const, entry_low: 3925, entry_high: 3928.5 };
		const [market, limit] = prepareChoices(sell, { bid: 3925.09, ask: 3925.22 });
		expect(market.price).toBe(3925.09);
		expect(limit.price).toBe(3928.5);
	});

	it('restates side, stop and TP1', () => {
		expect(setupLine(setup, 2)).toBe('Buy · stop 3,911.00 · TP1 3,944.10');
		expect(setupLine({ ...setup, side: 'sell' }, 2).startsWith('Sell')).toBe(true);
	});

	it('is offered only for a current view with a setup', () => {
		expect(canPrepare(view(), NOW)).toBe(true);
		expect(canPrepare(view({ setup: null }), NOW)).toBe(false);
		expect(canPrepare(view({ valid_until: at(-1) }), NOW)).toBe(false);
		expect(canPrepare(view({ stand_aside: true }), NOW)).toBe(false);
	});
});

describe('symbol box and chat labels', () => {
	it('cleans the symbol text', () => {
		expect(cleanSymbolInput('  xauusd ')).toBe('xauusd');
		expect(cleanSymbolInput('S&P   500')).toBe('S&P 500');
		expect(cleanSymbolInput('   ')).toBeNull();
		expect(cleanSymbolInput('x'.repeat(50))?.length).toBe(32);
	});

	it('gives the market tools friendly names', () => {
		expect(toolLabel('get_market_view')).toBe('Market view');
		expect(toolLabel('get_economic_calendar')).toBe('Economic calendar');
		expect(toolLabel('get_news')).toBe('News');
		expect(toolLabel('get_price')).toBe('get_price');
	});
});
