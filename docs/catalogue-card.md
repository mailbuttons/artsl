# Proposed catalogue card: the artist

A draft for `src/data/catalogue.ts` and the matching `agent_catalogue` seed
row in the mailbuttons repo (the rule there: seed and array change in the same
commit). Copy follows the store voice: British English, short sentences, no
exclamation marks, no developer words on buyer pages.

Decisions left open, marked `TODO`: the price (free with an account is the
simplest honest label, and what the service enforces today) and whether to
ship an email address for it.

```ts
{
  slug: 'artist', name: 'The artist', trade: 'Our crew', role: 'Console art',
  does: 'Draws a picture of the day from the news for your terminal. Subscribers can have one drawn on the spot.',
  approves: 'Nothing; it draws.', availability: 'hireable',
  price_label: 'Free with an account',           // TODO confirm
  cta_label: 'Get this agent', cta_href: '/agents/artist/', sort_order: 60,
  needs: 'The artsl command on your machine, and an email address to sign in with.',
  address_pattern: null,                          // TODO: art@mailbuttons.com could reply with the picture
  blurb:
    'Type artsl and a picture rolls across your terminal, drawn that morning from the news by the same model that runs our other agents. No account needed for that.\n\n' +
    'Sign in with a one-time code and artsl --fresh has one drawn for you on the spot, from whatever the news is right now. A couple of dozen a day. It keeps the headline and the publisher under every picture, so you always know what it was looking at.',
}
```

## Where it sits in the store

- Trade "Our crew" keeps it beside Quartermaster, Scribe and Herald: things we
  run that show what an agent on the platform looks like.
- The agent page is the natural home for the install line and the
  `artsl subscribe` line. Keep them in a code block; the rest of the page is
  for the buyer.
- Proof next to claim: embed today's picture on the page by fetching
  `/art/latest.json` (CORS is open) and rendering frame 0 in a `<pre>`.

## Follow-ups, not in this change

- An email address (`art@mailbuttons.com`): a hosted agent that replies to any
  approved sender with the picture of the day as plain text. Cheap to add with
  the hosted worker; it would make the artist an agent you can email, like the
  rest of the crew.
- Billing: today the service accepts any active mailbuttons key and applies a
  daily allowance. If the artist becomes a paid listing, the service's key
  check becomes an entitlement check. The command-line tool does not change.
