use futures::future::BoxFuture;
use futures::FutureExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::multipart::Form;
use reqwest::{Body, Client, IntoUrl, Method, Request, Response};
use serde::Serialize;
use std::convert::TryFrom;
use std::fmt::{self, Display};
use std::time::Duration;
use task_local_extensions::Extensions;
// use tower::{Layer, Service, ServiceBuilder, ServiceExt};

use crate::{Error, Identity, Layer, RequestInitialiser, RequestStack, Service, Stack};

/// A `ClientBuilder` is used to build a [`ClientWithMiddleware`].
///
/// [`ClientWithMiddleware`]: crate::ClientWithMiddleware
pub struct ClientBuilder<M, I> {
    client: Client,
    middleware_stack: M,
    initialiser_stack: I,
}

impl ClientBuilder<Identity, Identity> {
    pub fn new(client: Client) -> Self {
        ClientBuilder {
            client,
            middleware_stack: Identity,
            initialiser_stack: Identity,
        }
    }
}

impl<M, I> ClientBuilder<M, I> {
    /// Convenience method to attach middleware.
    pub fn with<T>(self, layer: T) -> ClientBuilder<Stack<T, M>, I> {
        ClientBuilder {
            client: self.client,
            middleware_stack: Stack {
                inner: layer,
                outer: self.middleware_stack,
            },
            initialiser_stack: self.initialiser_stack,
        }
    }

    /// Convenience method to attach a request initialiser.
    pub fn with_init<T>(self, initialiser: T) -> ClientBuilder<M, RequestStack<T, I>> {
        ClientBuilder {
            client: self.client,
            middleware_stack: self.middleware_stack,
            initialiser_stack: RequestStack {
                inner: initialiser,
                outer: self.initialiser_stack,
            },
        }
    }

    /// Returns a `ClientWithMiddleware` using this builder configuration.
    pub fn build(self) -> ClientWithMiddleware<M, I> {
        ClientWithMiddleware {
            inner: self.client,
            middleware_stack: self.middleware_stack,
            initialiser_stack: self.initialiser_stack,
        }
    }
}

/// `ClientWithMiddleware` is a wrapper around [`reqwest::Client`] which runs middleware on every
/// request.
#[derive(Clone)]
pub struct ClientWithMiddleware<M, I> {
    inner: reqwest::Client,
    middleware_stack: M,
    initialiser_stack: I,
}

impl<M: Layer<ReqwestService>, I: RequestInitialiser> ClientWithMiddleware<M, I> {
    /// See [`Client::get`]
    pub fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder<M, I> {
        self.request(Method::GET, url)
    }

    /// See [`Client::post`]
    pub fn post<U: IntoUrl>(&self, url: U) -> RequestBuilder<M, I> {
        self.request(Method::POST, url)
    }

    /// See [`Client::put`]
    pub fn put<U: IntoUrl>(&self, url: U) -> RequestBuilder<M, I> {
        self.request(Method::PUT, url)
    }

    /// See [`Client::patch`]
    pub fn patch<U: IntoUrl>(&self, url: U) -> RequestBuilder<M, I> {
        self.request(Method::PATCH, url)
    }

    /// See [`Client::delete`]
    pub fn delete<U: IntoUrl>(&self, url: U) -> RequestBuilder<M, I> {
        self.request(Method::DELETE, url)
    }

    /// See [`Client::head`]
    pub fn head<U: IntoUrl>(&self, url: U) -> RequestBuilder<M, I> {
        self.request(Method::HEAD, url)
    }

    /// See [`Client::request`]
    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> RequestBuilder<'_, M, I> {
        let mut extensions = Extensions::new();
        let request = self.inner.request(method, url);
        let request = self.initialiser_stack.init(request, &mut extensions);
        RequestBuilder {
            inner: request,
            client: self,
            extensions,
        }
    }
}

/// Create a `ClientWithMiddleware` without any middleware.
impl From<Client> for ClientWithMiddleware<Identity, Identity> {
    fn from(client: Client) -> Self {
        ClientWithMiddleware {
            inner: client,
            middleware_stack: Identity,
            initialiser_stack: Identity,
        }
    }
}

impl<M, I> fmt::Debug for ClientWithMiddleware<M, I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // skipping middleware_stack field for now
        f.debug_struct("ClientWithMiddleware")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

/// This is a wrapper around [`reqwest::RequestBuilder`] exposing the same API.
#[must_use = "RequestBuilder does nothing until you 'send' it"]
pub struct RequestBuilder<'client, M, I> {
    inner: reqwest::RequestBuilder,
    client: &'client ClientWithMiddleware<M, I>,
    extensions: Extensions,
}

#[derive(Clone)]
pub struct ReqwestService(Client);

impl Service for ReqwestService {
    type Future = BoxFuture<'static, Result<Response, Error>>;

    fn call(&mut self, req: Request, _: &mut Extensions) -> Self::Future {
        let fut = self.0.execute(req);
        async { fut.await.map_err(Error::from) }.boxed()
    }
}

impl<M: Layer<ReqwestService>, I: RequestInitialiser> RequestBuilder<'_, M, I>
where
    M::Service: Service,
{
    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        RequestBuilder {
            inner: self.inner.header(key, value),
            ..self
        }
    }

    pub fn headers(self, headers: HeaderMap) -> Self {
        RequestBuilder {
            inner: self.inner.headers(headers),
            ..self
        }
    }

    pub fn basic_auth<U, P>(self, username: U, password: Option<P>) -> Self
    where
        U: Display,
        P: Display,
    {
        RequestBuilder {
            inner: self.inner.basic_auth(username, password),
            ..self
        }
    }

    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: Display,
    {
        RequestBuilder {
            inner: self.inner.bearer_auth(token),
            ..self
        }
    }

    pub fn body<T: Into<Body>>(self, body: T) -> Self {
        RequestBuilder {
            inner: self.inner.body(body),
            ..self
        }
    }

    pub fn timeout(self, timeout: Duration) -> Self {
        RequestBuilder {
            inner: self.inner.timeout(timeout),
            ..self
        }
    }

    pub fn multipart(self, multipart: Form) -> Self {
        RequestBuilder {
            inner: self.inner.multipart(multipart),
            ..self
        }
    }

    pub fn query<T: Serialize + ?Sized>(self, query: &T) -> Self {
        RequestBuilder {
            inner: self.inner.query(query),
            ..self
        }
    }

    pub fn form<T: Serialize + ?Sized>(self, form: &T) -> Self {
        RequestBuilder {
            inner: self.inner.form(form),
            ..self
        }
    }

    pub fn json<T: Serialize + ?Sized>(self, json: &T) -> Self {
        RequestBuilder {
            inner: self.inner.json(json),
            ..self
        }
    }

    pub fn build(self) -> reqwest::Result<Request> {
        self.inner.build()
    }

    /// Inserts the extension into this request builder
    pub fn with_extension<T: Send + Sync + 'static>(mut self, extension: T) -> Self {
        self.extensions.insert(extension);
        self
    }

    /// Returns a mutable reference to the internal set of extensions for this request
    pub fn extensions(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    pub async fn send(self) -> Result<Response, Error> {
        let Self {
            inner,
            client,
            mut extensions,
        } = self;
        let req = inner.build()?;
        let mut svc = client
            .middleware_stack
            .layer(ReqwestService(client.inner.clone()));
        svc.call(req, &mut extensions).await

        // client.execute_with_extensions(req, &mut extensions).await
    }

    /// Attempt to clone the RequestBuilder.
    ///
    /// `None` is returned if the RequestBuilder can not be cloned,
    /// i.e. if the request body is a stream.
    ///
    /// # Extensions
    /// Note that extensions are not preserved through cloning.
    pub fn try_clone(&self) -> Option<Self> {
        self.inner.try_clone().map(|inner| RequestBuilder {
            inner,
            client: self.client,
            extensions: Extensions::new(),
        })
    }
}

impl<M, I> fmt::Debug for RequestBuilder<'_, M, I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // skipping middleware_stack field for now
        f.debug_struct("RequestBuilder")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}
