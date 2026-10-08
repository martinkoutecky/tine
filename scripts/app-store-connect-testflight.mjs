#!/usr/bin/env node

import { createPrivateKey, sign } from "node:crypto";

const API = "https://api.appstoreconnect.apple.com/v1";
const bundleId = process.env.APP_BUNDLE_ID || "page.tine.Tine";
const command = process.argv[2] || "inspect";
const wantedVersion = process.env.TESTFLIGHT_BUILD_NUMBER || "";
const groupName = process.env.TESTFLIGHT_GROUP_NAME || "Tine iOS Public Beta";

function required(name) {
  const value = process.env[name];
  if (!value) throw new Error(`missing required environment variable ${name}`);
  return value;
}

function base64url(value) {
  return Buffer.from(value).toString("base64url");
}

function token() {
  const now = Math.floor(Date.now() / 1000);
  const header = base64url(
    JSON.stringify({
      alg: "ES256",
      kid: required("APPLE_API_KEY"),
      typ: "JWT",
    }),
  );
  const payload = base64url(
    JSON.stringify({
      iss: required("APPLE_API_ISSUER"),
      iat: now,
      exp: now + 15 * 60,
      aud: "appstoreconnect-v1",
    }),
  );
  const input = `${header}.${payload}`;
  const signature = sign("sha256", Buffer.from(input), {
    key: createPrivateKey(required("APPLE_API_PRIVATE_KEY")),
    dsaEncoding: "ieee-p1363",
  }).toString("base64url");
  return `${input}.${signature}`;
}

async function request(method, path, body) {
  const response = await fetch(`${API}${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${token()}`,
      ...(body ? { "Content-Type": "application/json" } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  let parsed;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = { raw: text };
  }
  if (!response.ok) {
    const details =
      parsed?.errors?.map((error) => ({
        status: error.status,
        code: error.code,
        title: error.title,
        detail: error.detail,
      })) ?? parsed;
    throw new Error(
      `${method} ${path} failed (${response.status}): ${JSON.stringify(details)}`,
    );
  }
  return parsed;
}

function query(params) {
  return new URLSearchParams(params).toString();
}

async function app() {
  const response = await request(
    "GET",
    `/apps?${query({
      "filter[bundleId]": bundleId,
      "fields[apps]": "name,bundleId,primaryLocale",
      limit: "2",
    })}`,
  );
  if (response.data.length !== 1) {
    throw new Error(
      `expected one App Store Connect app for ${bundleId}, found ${response.data.length}`,
    );
  }
  return response.data[0];
}

async function latestBuild(appId, { wait = false } = {}) {
  const deadline = Date.now() + 25 * 60 * 1000;
  for (;;) {
    const filters = {
      "filter[app]": appId,
      "fields[builds]":
        "version,uploadedDate,processingState,expired,usesNonExemptEncryption",
      sort: "-uploadedDate",
      limit: "20",
    };
    if (wantedVersion) filters["filter[version]"] = wantedVersion;
    const response = await request("GET", `/builds?${query(filters)}`);
    const build = response.data[0];
    if (!build)
      throw new Error(
        `no uploaded build found${wantedVersion ? ` with build number ${wantedVersion}` : ""}`,
      );
    if (!wait || build.attributes.processingState !== "PROCESSING")
      return build;
    if (Date.now() >= deadline)
      throw new Error(
        `build ${build.attributes.version} is still processing after 25 minutes`,
      );
    console.log(
      `build ${build.attributes.version} is still processing; retrying in 30 seconds`,
    );
    await new Promise((resolve) => setTimeout(resolve, 30_000));
  }
}

async function list(path, params) {
  return (await request("GET", `${path}?${query(params)}`)).data;
}

async function reviewDetail(appId) {
  return (
    await request(
      "GET",
      `/apps/${appId}/betaAppReviewDetail?${query({
        "fields[betaAppReviewDetails]":
          "contactFirstName,contactLastName,contactPhone,contactEmail,demoAccountRequired,notes",
      })}`,
    )
  ).data;
}

function missingReviewContact(detail) {
  const attrs = detail.attributes || {};
  return [
    "contactFirstName",
    "contactLastName",
    "contactPhone",
    "contactEmail",
  ].filter((field) => !attrs[field]);
}

async function state({ wait = false, proveMembership = false } = {}) {
  const appResource = await app();
  const build = await latestBuild(appResource.id, { wait });
  const [localizations, groups, review, submissions] = await Promise.all([
    list("/betaAppLocalizations", {
      "filter[app]": appResource.id,
      "fields[betaAppLocalizations]":
        "locale,description,feedbackEmail,marketingUrl,privacyPolicyUrl",
      limit: "50",
    }),
    list("/betaGroups", {
      "filter[app]": appResource.id,
      "fields[betaGroups]":
        "name,isInternalGroup,publicLinkEnabled,publicLink,publicLinkLimitEnabled,publicLinkLimit",
      limit: "50",
    }),
    reviewDetail(appResource.id),
    list("/betaAppReviewSubmissions", {
      "filter[build]": build.id,
      "fields[betaAppReviewSubmissions]": "betaReviewState,submittedDate",
      limit: "20",
    }),
  ]);
  const publicGroup = groups.find(
    (item) =>
      !item.attributes.isInternalGroup && item.attributes.name === groupName,
  );
  let publicGroupBuildIds = null;
  if (proveMembership && publicGroup) {
    const attached = await request(
      "GET",
      `/betaGroups/${publicGroup.id}/relationships/builds?${query({ limit: "200" })}`,
    );
    if (attached.links?.next) {
      throw new Error("public beta group build list exceeds one page; cannot prove membership");
    }
    publicGroupBuildIds = attached.data.map((item) => item.id);
  }
  return {
    appResource,
    build,
    localizations,
    groups,
    review,
    submissions,
    publicGroupBuildIds,
  };
}

function safeSummary(current) {
  return {
    app: {
      id: current.appResource.id,
      name: current.appResource.attributes.name,
      bundleId: current.appResource.attributes.bundleId,
      primaryLocale: current.appResource.attributes.primaryLocale,
    },
    build: { id: current.build.id, ...current.build.attributes },
    betaLocalizations: current.localizations.map(({ id, attributes }) => ({
      id,
      ...attributes,
    })),
    groups: current.groups.map(({ id, attributes }) => ({ id, ...attributes })),
    selectedBuildInPublicBetaGroup:
      current.publicGroupBuildIds === null
        ? null
        : current.publicGroupBuildIds.includes(current.build.id),
    reviewContactComplete: missingReviewContact(current.review).length === 0,
    missingReviewContactFields: missingReviewContact(current.review),
    submissions: current.submissions.map(({ id, attributes }) => ({
      id,
      ...attributes,
    })),
  };
}

async function upsertAppLocalization(appId, existing) {
  const attributes = {
    locale: "en-US",
    description:
      "Tine is a fast, local-first outliner compatible with Logseq Markdown and Org graphs. This early iOS beta focuses on opening, editing, and safely preserving Tine-owned graphs in On My iPhone/iPad and iCloud Drive.",
    feedbackEmail: "support@tine.page",
    marketingUrl: "https://tine.page/",
    privacyPolicyUrl: "https://tine.page/privacy.html",
  };
  if (existing) {
    delete attributes.locale;
    return (
      await request("PATCH", `/betaAppLocalizations/${existing.id}`, {
        data: { type: "betaAppLocalizations", id: existing.id, attributes },
      })
    ).data;
  }
  return (
    await request("POST", "/betaAppLocalizations", {
      data: {
        type: "betaAppLocalizations",
        attributes,
        relationships: { app: { data: { type: "apps", id: appId } } },
      },
    })
  ).data;
}

async function upsertBuildLocalization(buildId) {
  const existing = await list("/betaBuildLocalizations", {
    "filter[build]": buildId,
    "filter[locale]": "en-US",
    "fields[betaBuildLocalizations]": "locale,whatsNew",
    limit: "10",
  });
  const attributes = {
    whatsNew:
      "Please test first-run graph creation, local and iCloud Drive graphs, editing and saving, backgrounding and force-closing the app, reopening graphs, and clear handling of unsupported Files-provider locations. This is an early beta; please report any data-safety or recovery problem immediately.",
  };
  if (existing[0]) {
    return (
      await request("PATCH", `/betaBuildLocalizations/${existing[0].id}`, {
        data: {
          type: "betaBuildLocalizations",
          id: existing[0].id,
          attributes,
        },
      })
    ).data;
  }
  return (
    await request("POST", "/betaBuildLocalizations", {
      data: {
        type: "betaBuildLocalizations",
        attributes: { locale: "en-US", ...attributes },
        relationships: { build: { data: { type: "builds", id: buildId } } },
      },
    })
  ).data;
}

async function upsertGroup(appId, existing) {
  if (existing) return existing;
  return (
    await request("POST", "/betaGroups", {
      data: {
        type: "betaGroups",
        attributes: {
          name: groupName,
          isInternalGroup: false,
          feedbackEnabled: true,
          publicLinkEnabled: false,
        },
        relationships: { app: { data: { type: "apps", id: appId } } },
      },
    })
  ).data;
}

async function addBuild(groupId, buildId) {
  await request("POST", `/betaGroups/${groupId}/relationships/builds`, {
    data: [{ type: "builds", id: buildId }],
  });
}

async function prepare() {
  const current = await state({ wait: true });
  if (current.build.attributes.processingState !== "VALID") {
    throw new Error(
      `build ${current.build.attributes.version} is ${current.build.attributes.processingState}, not VALID`,
    );
  }
  const locale =
    current.localizations.find((item) => item.attributes.locale === "en-US") ||
    current.localizations.find(
      (item) =>
        item.attributes.locale === current.appResource.attributes.primaryLocale,
    );
  await upsertAppLocalization(current.appResource.id, locale);
  await upsertBuildLocalization(current.build.id);
  const group = await upsertGroup(
    current.appResource.id,
    current.groups.find(
      (item) =>
        !item.attributes.isInternalGroup && item.attributes.name === groupName,
    ),
  );
  await addBuild(group.id, current.build.id);

  const reviewPhone = process.env.APPLE_REVIEW_CONTACT_PHONE;
  const reviewAttributes = {
    contactFirstName: "Martin",
    contactLastName: "Koutecky",
    contactEmail: "support@tine.page",
    demoAccountRequired: false,
    notes:
      "Tine is a local-first outliner. No account or login is required. For the first beta, create or choose a TineOutline-owned graph under On My iPhone/iPad or iCloud Drive; arbitrary third-party Files-provider roots are intentionally unsupported.",
  };
  if (reviewPhone) reviewAttributes.contactPhone = reviewPhone;
  await request("PATCH", `/betaAppReviewDetails/${current.review.id}`, {
    data: {
      type: "betaAppReviewDetails",
      id: current.review.id,
      attributes: reviewAttributes,
    },
  });
  console.log(JSON.stringify(safeSummary(await state()), null, 2));
}

async function submit() {
  const current = await state({ wait: true });
  const missing = missingReviewContact(current.review);
  if (missing.length)
    throw new Error(
      `beta review contact is incomplete; missing: ${missing.join(", ")}`,
    );
  if (
    !current.localizations.length ||
    current.localizations.some((item) => !item.attributes.description)
  ) {
    throw new Error(
      "every beta app localization must have a description before review submission",
    );
  }
  const prior = current.submissions.find(
    (item) =>
      !["REJECTED", "CANCELED"].includes(item.attributes.betaReviewState),
  );
  if (prior) {
    console.log(
      `review submission already exists in state ${prior.attributes.betaReviewState}`,
    );
    console.log(JSON.stringify(safeSummary(current), null, 2));
    return;
  }
  await request("POST", "/betaAppReviewSubmissions", {
    data: {
      type: "betaAppReviewSubmissions",
      relationships: {
        build: { data: { type: "builds", id: current.build.id } },
      },
    },
  });
  console.log(JSON.stringify(safeSummary(await state()), null, 2));
}

async function publishLink() {
  const current = await state();
  const accepted = current.submissions.some(
    (item) => item.attributes.betaReviewState === "APPROVED",
  );
  if (!accepted) {
    const states =
      current.submissions
        .map((item) => item.attributes.betaReviewState)
        .join(", ") || "none";
    throw new Error(`beta review is not approved; current state: ${states}`);
  }
  const group = current.groups.find(
    (item) =>
      !item.attributes.isInternalGroup && item.attributes.name === groupName,
  );
  if (!group) throw new Error(`external group ${groupName} does not exist`);
  await request("PATCH", `/betaGroups/${group.id}`, {
    data: {
      type: "betaGroups",
      id: group.id,
      attributes: {
        publicLinkEnabled: true,
        publicLinkLimitEnabled: true,
        publicLinkLimit: 100,
      },
    },
  });
  const updated = await state();
  const publicGroup = updated.groups.find((item) => item.id === group.id);
  const link = publicGroup?.attributes.publicLink;
  if (!link)
    throw new Error(
      "Apple enabled the public group but did not return a public link",
    );
  console.log(`TESTFLIGHT_PUBLIC_LINK=${link}`);
  if (process.env.GITHUB_OUTPUT) {
    const { appendFile } = await import("node:fs/promises");
    await appendFile(process.env.GITHUB_OUTPUT, `public_link=${link}\n`);
  }
}

switch (command) {
  case "inspect":
    console.log(
      JSON.stringify(safeSummary(await state({ proveMembership: true })), null, 2),
    );
    break;
  case "prepare":
    await prepare();
    break;
  case "submit":
    await submit();
    break;
  case "publish-link":
    await publishLink();
    break;
  default:
    throw new Error(`unknown command ${command}`);
}
